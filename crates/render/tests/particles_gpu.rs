#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::too_many_lines, reason = "the lavapipe agreement, the cap, and the negative control share one device"))]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "the sync child is this test binary; the parent selects the mode with an env var so it can inject the validation layer"
    )
)]

//! Stage-1 compute on lavapipe against the f64 mirror.
//!
//! Positions may differ by 1e-3 m. Alive counts match. A step that drops `dt`
//! must miss that gate. Skipping the indirect count draws nothing.

use genos_render::{
    CpuParticles, ParticleFault, Reflector, Renderer, StepInput, World, LOW_VRAM_BYTES,
};
use genos_scene::{
    view_proj, Camera, Floor, LightingPreset, LightingSettings, ParticleSettings, Scene, Vec3, Wall,
    DEFAULT_SEED,
};
use genos_window::{Lifecycle, Window};

const DT: f32 = 1.0 / 60.0;
const POS_ABS: f64 = 1.0e-3;

fn open() -> (u32, u32, Lifecycle<Renderer>) {
    let mut window = Window::open_proof(64, 64).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (
        frame.width,
        frame.height,
        Lifecycle::new(window, renderer, permit),
    )
}

fn world(particles: ParticleSettings) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [0.2, 0.2, 0.2],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::Potato),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
        particles,
    })
}

fn emitter(max_particles: u32, count: u32) -> ParticleSettings {
    let mut settings = ParticleSettings::preset(genos_scene::ParticlePreset::Custom);
    settings.enabled = true;
    settings.max_particles = max_particles;
    settings.count = count;
    settings.rate = 30.0;
    settings.life = 2.0;
    settings.rise = 0.4;
    settings.spread = 0.25;
    settings.size = 0.2;
    settings.seed = DEFAULT_SEED;
    settings.origin = [0.0, 1.7, 0.0];
    settings.albedo = [0.1, 0.2, 0.9];
    settings
}

fn step_cpu(
    cpu: &mut CpuParticles,
    settings: &ParticleSettings,
    aspect: f32,
    camera: &Camera,
    depth_w: u32,
    depth_h: u32,
    depth: Vec<f32>,
) {
    let mut input = StepInput::from_settings(
        settings,
        DT,
        view_proj(camera, aspect),
        depth_w,
        depth_h,
        depth,
    );
    input.skip_dead = false;
    input.skip_indirect = false;
    cpu.step(&input);
}

fn agree(gpu: &[[f32; 3]], cpu: &[[f64; 3]]) -> f64 {
    assert_eq!(gpu.len(), cpu.len(), "alive lists differ");
    let mut peak = 0.0_f64;
    for (got, want) in gpu.iter().zip(cpu) {
        for axis in 0..3 {
            peak = peak.max((f64::from(got[axis]) - want[axis]).abs());
        }
    }
    peak
}

fn draw(
    life: &mut Lifecycle<Renderer>,
    world: &World,
    camera: &Camera,
) -> Vec<u8> {
    let _ = life.window.pump();
    life.device.set_particle_dt(DT);
    life.device
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn center_blue(pixels: &[u8], width: u32, height: u32) -> u8 {
    let x = width / 2;
    let y = height / 2;
    let index = (y * width + x) * 4;
    pixels.get(index as usize).copied().unwrap_or(0)
}

#[test]
fn gpu_matches_the_cpu_mirror_and_the_cap_holds() {
    let (width, height, mut life) = open();
    let aspect = width as f32 / height.max(1) as f32;
    let camera = Camera::new(0.0, 2.0, 0.0);
    let mut settings = emitter(16, 8);
    let mut cpu = CpuParticles::new();
    let mut first = Vec::new();
    for _ in 0..5 {
        let drawn = world(settings.clone());
        draw(&mut life, &drawn, &camera);
        step_cpu(&mut cpu, &settings, aspect, &camera, 0, 0, Vec::new());
        let gpu = life.device.particle_positions().expect("positions");
        let peak = agree(&gpu, cpu.positions());
        assert!(peak <= POS_ABS, "gpu drifted {peak} m from the f64 mirror");
        assert_eq!(life.device.particle_stats().alive, cpu.alive());
        first = gpu;
    }
    assert!(cpu.alive() > 0, "the emitter never spawned");
    let stats = life.device.particle_stats();
    assert!(stats.sim_ms > 0.0, "sim timestamp was not written");
    assert!(stats.vram_bytes > 0);
    assert!(stats.vram_bytes <= LOW_VRAM_BYTES);
    assert!(stats.line().contains("fx.alive="));
    assert_eq!(stats.sort_ms, 0.0);
    assert_eq!(stats.overdraw_px, 0);

    let mut broken = CpuParticles::new();
    let mut dropped = settings.clone();
    dropped.freeze = true;
    for _ in 0..5 {
        step_cpu(&mut broken, &dropped, aspect, &camera, 0, 0, Vec::new());
    }
    assert_eq!(broken.alive(), 0, "a frozen step must not spawn");
    assert_ne!(
        broken.positions(),
        cpu.positions(),
        "dropping motion must miss the agreement gate"
    );

    settings.seed = DEFAULT_SEED.wrapping_add(1);
    draw(&mut life, &world(settings.clone()), &camera);
    settings.seed = DEFAULT_SEED;
    let mut again = CpuParticles::new();
    for _ in 0..5 {
        draw(&mut life, &world(settings.clone()), &camera);
        step_cpu(&mut again, &settings, aspect, &camera, 0, 0, Vec::new());
    }
    let gpu = life.device.particle_positions().expect("replay");
    let peak = agree(&gpu, again.positions());
    assert!(peak <= POS_ABS, "replay drifted {peak}");
    assert_eq!(gpu, first, "the same seed did not replay");

    let mut capped = emitter(4, 64);
    capped.rate = 240.0;
    capped.life = 5.0;
    let mut cap_cpu = CpuParticles::new();
    for _ in 0..6 {
        draw(&mut life, &world(capped.clone()), &camera);
        step_cpu(&mut cap_cpu, &capped, aspect, &camera, 0, 0, Vec::new());
        let gpu = life.device.particle_positions().expect("cap");
        let peak = agree(&gpu, cap_cpu.positions());
        assert!(peak <= POS_ABS, "cap drifted {peak}");
        assert!(cap_cpu.alive() <= 4, "alive {}", cap_cpu.alive());
        assert_eq!(life.device.particle_stats().alive, cap_cpu.alive());
    }
    assert_eq!(cap_cpu.spawned() - cap_cpu.died(), cap_cpu.alive());

    let mut card = emitter(4, 1);
    card.seed = DEFAULT_SEED.wrapping_add(2);
    card.rate = 60.0;
    card.spread = 0.0;
    card.rise = 0.0;
    card.size = 8.0;
    card.albedo = [0.0, 0.0, 1.0];
    card.life = 10.0;
    life.device.set_particle_fault(ParticleFault::None);
    let pixels = draw(&mut life, &world(card.clone()), &camera);
    let shown = center_blue(&pixels, width, height);
    assert!(shown > 200, "centre blue {shown}, the card was not drawn");
    life.device.set_particle_fault(ParticleFault::SkipIndirect);
    card.rate = 0.0;
    let hidden = draw(&mut life, &world(card.clone()), &camera);
    let skipped = center_blue(&hidden, width, height);
    assert!(
        skipped < shown / 2,
        "skip-indirect still drew blue {skipped} (was {shown})"
    );
    let after = life.device.particle_stats();
    assert_eq!(after.draw_instances, 0);
    assert!(after.alive > 0, "the particle should still be alive");
    life.device.set_particle_fault(ParticleFault::None);

    let mut depth_settings = emitter(4, 1);
    depth_settings.seed = DEFAULT_SEED.wrapping_add(3);
    depth_settings.spread = 0.0;
    depth_settings.rise = 0.0;
    depth_settings.rate = 60.0;
    depth_settings.life = 10.0;
    depth_settings.depth_collision = true;
    draw(&mut life, &world(depth_settings.clone()), &camera);
    assert_eq!(life.device.particle_stats().alive, 1);
    depth_settings.rate = 0.0;
    life.device.set_particle_depth(1, 1, &[0.0]);
    let mut depth_cpu = CpuParticles::new();
    step_cpu(
        &mut depth_cpu,
        &depth_settings,
        aspect,
        &camera,
        0,
        0,
        Vec::new(),
    );
    // The GPU already integrated the spawn frame. Catch the mirror up, then collide.
    depth_settings.rate = 60.0;
    depth_cpu = CpuParticles::new();
    step_cpu(
        &mut depth_cpu,
        &depth_settings,
        aspect,
        &camera,
        0,
        0,
        Vec::new(),
    );
    depth_settings.rate = 0.0;
    step_cpu(
        &mut depth_cpu,
        &depth_settings,
        aspect,
        &camera,
        1,
        1,
        vec![0.0],
    );
    draw(&mut life, &world(depth_settings), &camera);
    let gpu = life.device.particle_positions().expect("depth");
    let peak = agree(&gpu, depth_cpu.positions());
    assert!(peak <= POS_ABS, "depth step drifted {peak}");
    assert_eq!(
        life.device.particle_stats().alive, 0,
        "a particle in front of the camera is behind depth 0"
    );
    let depth_bytes = u64::from(width).saturating_mul(u64::from(height)) * 8;
    let vram = life.device.particle_stats().vram_bytes;
    assert!(
        vram >= depth_bytes,
        "vram {vram} omitted the two retained depth images ({depth_bytes})"
    );
    eprintln!(
        "vram_bytes {vram} vram_mb {:.3} depth_images {depth_bytes}",
        vram as f64 / 1_000_000.0
    );
}

#[test]
fn the_sim_dispatch_is_wider_than_one_invocation() {
    let (width, height, mut life) = open();
    let aspect = width as f32 / height.max(1) as f32;
    let camera = Camera::new(0.0, 2.0, 0.0);
    // 96 fills workgroup 0 and 32 slots of workgroup 1. A later group's
    // block exclusive is the earlier group's count, and a slot after the
    // first of that group has a non-zero local exclusive. Life is two steps,
    // so the third frame retires both groups in one dead-list scan.
    // 8192 slots is 128 block sums, so the block scan itself is more than one
    // workgroup. 96 live slots still sit past the first particle workgroup.
    let mut settings = emitter(8192, 96);
    settings.rate = 1.0e6;
    settings.life = 2.0 * DT;
    let mut cpu = CpuParticles::new();
    let mut saw_later_live = false;
    for _ in 0..3 {
        draw(&mut life, &world(settings.clone()), &camera);
        step_cpu(&mut cpu, &settings, aspect, &camera, 0, 0, Vec::new());
        let gpu = life.device.particle_positions().expect("positions");
        let peak = agree(&gpu, cpu.positions());
        assert!(
            peak <= POS_ABS,
            "gpu drifted {peak} m from the f64 mirror"
        );
        assert_eq!(life.device.particle_stats().alive, cpu.alive());
        if cpu.alive() > 65 {
            saw_later_live = true;
        }
    }
    assert!(
        saw_later_live,
        "no live slot past the first workgroup"
    );
    assert!(
        cpu.died() > 65,
        "no death in a later workgroup for the block exclusive add"
    );
    let (size, groups) = life.device.particle_sim_launch().expect("launch");
    assert!(size > 1, "sim workgroup was a single thread");
    assert!(groups > 1, "sim dispatch was a single group");
    assert!(
        u64::from(size) * u64::from(groups) >= u64::from(settings.max_particles),
        "sim launch {size}x{groups} does not cover the budget"
    );
    let (scan_size, scan_groups) = life.device.particle_scan_launch().expect("scan launch");
    assert!(scan_size > 1, "block scan workgroup was a single thread");
    assert!(
        scan_groups > 1,
        "block scan was one group, so one thread still walks the sums"
    );
    let parts = life.device.particle_sim_parts();
    eprintln!(
        "sim_parts init={:.3} death={:.3} emit={:.3} compact={:.3} sim={:.3}",
        parts[0], parts[1], parts[2], parts[3], life.device.particle_stats().sim_ms
    );
}

#[test]
fn a_different_dt_and_a_skipped_dead_ring_match_on_device() {
    let (width, height, mut life) = open();
    let aspect = width as f32 / height.max(1) as f32;
    let camera = Camera::new(0.0, 2.0, 0.0);
    let mut settings = emitter(8, 8);
    settings.life = 1.0;
    settings.rate = 20.0;
    let mut cpu = CpuParticles::new();
    let mut slow = Vec::new();
    for frame in 0..6 {
        let dt = if frame < 3 { DT } else { 0.1 };
        life.device.set_particle_dt(dt);
        let _ = life.window.pump();
        life.device
            .draw(&world(settings.clone()), &camera, true)
            .expect("draw")
            .expect("readback");
        let input = StepInput::from_settings(
            &settings,
            dt,
            view_proj(&camera, aspect),
            0,
            0,
            Vec::new(),
        );
        cpu.step(&input);
        let gpu = life.device.particle_positions().expect("positions");
        let peak = agree(&gpu, cpu.positions());
        assert!(peak <= POS_ABS, "dt {dt} drifted {peak}");
        assert_eq!(life.device.particle_stats().alive, cpu.alive());
        if frame == 5 {
            assert_ne!(gpu, slow, "a larger dt did not move the sim");
        } else if frame == 2 {
            slow = gpu;
        }
    }

    life.device.set_particle_dt(f32::NAN);
    let before = life.device.particle_positions().expect("before nan");
    let mut nan_cpu = cpu.clone();
    for _ in 0..3 {
        let _ = life.window.pump();
        life.device
            .draw(&world(settings.clone()), &camera, true)
            .expect("nan draw")
            .expect("readback");
        let input = StepInput::from_settings(
            &settings,
            f32::NAN,
            view_proj(&camera, aspect),
            0,
            0,
            Vec::new(),
        );
        nan_cpu.step(&input);
    }
    let gpu = life.device.particle_positions().expect("nan");
    assert_eq!(gpu, before, "a non-finite dt moved the device sim");
    let peak = agree(&gpu, nan_cpu.positions());
    assert!(peak <= POS_ABS, "nan dt drifted {peak}");

    life.device.set_particle_dt(f32::INFINITY);
    let mut inf_cpu = nan_cpu.clone();
    for _ in 0..3 {
        let _ = life.window.pump();
        life.device
            .draw(&world(settings.clone()), &camera, true)
            .expect("inf draw")
            .expect("readback");
        let input = StepInput::from_settings(
            &settings,
            f32::INFINITY,
            view_proj(&camera, aspect),
            0,
            0,
            Vec::new(),
        );
        inf_cpu.step(&input);
    }
    let gpu = life.device.particle_positions().expect("inf");
    assert_eq!(gpu, before, "+inf dt moved the device sim");
    let peak = agree(&gpu, inf_cpu.positions());
    assert!(peak <= POS_ABS, "+inf dt drifted {peak}");

    let mut leak = emitter(2, 2);
    leak.seed = DEFAULT_SEED.wrapping_add(4);
    leak.life = 0.05;
    leak.rate = 60.0;
    life.device.set_particle_fault(ParticleFault::SkipDeadPush);
    life.device.set_particle_dt(DT);
    let mut leaked = CpuParticles::new();
    for _ in 0..40 {
        let _ = life.window.pump();
        life.device
            .draw(&world(leak.clone()), &camera, true)
            .expect("leak draw")
            .expect("readback");
        let mut input = StepInput::from_settings(
            &leak,
            DT,
            view_proj(&camera, aspect),
            0,
            0,
            Vec::new(),
        );
        input.skip_dead = true;
        leaked.step(&input);
        let gpu = life.device.particle_positions().expect("leak");
        let peak = agree(&gpu, leaked.positions());
        assert!(peak <= POS_ABS, "skip-dead drifted {peak}");
        assert_eq!(life.device.particle_stats().alive, leaked.alive());
    }
    assert_eq!(leaked.spawned(), 2, "the dead ring refilled");
    assert_eq!(life.device.particle_stats().spawned, 2);

    life.device.set_particle_fault(ParticleFault::None);
    leak.seed = DEFAULT_SEED.wrapping_add(5);
    let mut healthy = CpuParticles::new();
    for _ in 0..40 {
        let _ = life.window.pump();
        life.device
            .draw(&world(leak.clone()), &camera, true)
            .expect("healthy draw")
            .expect("readback");
        step_cpu(&mut healthy, &leak, aspect, &camera, 0, 0, Vec::new());
    }
    assert!(
        healthy.spawned() > 2,
        "the control did not refill, spawned {}",
        healthy.spawned()
    );
    assert_eq!(life.device.particle_stats().spawned, healthy.spawned());
}

#[test]
fn frames_without_readback_match_the_mirror() {
    let (width, height, mut life) = open();
    let aspect = width as f32 / height.max(1) as f32;
    let camera = Camera::new(0.0, 2.0, 0.0);
    let mut settings = emitter(16, 8);
    settings.spread = 0.4;
    let mut cpu = CpuParticles::new();
    for _ in 0..6 {
        let _ = life.window.pump();
        life.device.set_particle_dt(DT);
        life.device
            .draw_with_overlay(&world(settings.clone()), &camera, &[], false, false)
            .expect("submit");
        step_cpu(&mut cpu, &settings, aspect, &camera, 0, 0, Vec::new());
    }
    life.device.finish_particles().expect("finish");
    let gpu = life.device.particle_positions().expect("positions");
    let peak = agree(&gpu, cpu.positions());
    assert!(peak <= POS_ABS, "unread frames drifted {peak}");
    assert_eq!(life.device.particle_stats().alive, cpu.alive());
    assert!(life.device.particle_stats().sim_ms > 0.0, "completed flight had no sim stamp");
    assert!(cpu.alive() > 0);
}

/// Submits back to back. `draw_with_overlay` waits only the slot it reuses, so
/// frame N+1 is recorded while frame N can still be on the GPU.
fn overlap(life: &mut Lifecycle<Renderer>, settings: &ParticleSettings, camera: &Camera, frames: u32) {
    for _ in 0..frames {
        let _ = life.window.pump();
        life.device.set_particle_dt(DT);
        life.device
            .draw_with_overlay(&world(settings.clone()), camera, &[], false, false)
            .expect("submit");
    }
    life.device.finish_particles().expect("finish");
}

#[test]
fn the_depth_image_keeps_a_particle_in_front_and_retires_one_behind() {
    let (_w, _h, mut life) = open();
    let mut camera = Camera::new(0.0, 2.0, 0.0);
    camera.set_pose(Vec3::new(0.0, 1.7, 2.0), 0.0, -std::f32::consts::FRAC_PI_2);
    life.device.suspend_lighting();
    let mut settings = emitter(4, 2);
    settings.spread = 0.0;
    settings.rise = 0.0;
    settings.rate = 60.0;
    settings.life = 10.0;
    settings.depth_collision = true;
    // Straight down. This point sits between the eye and the floor.
    settings.origin = [0.0, 0.8, 2.0];
    let mut drawn = world(settings.clone());
    drawn.scene.walls.push(Wall {
        position: Vec3::new(0.0, 0.0, 1.2),
        half_x: 2.0,
        half_z: 0.2,
        height: 3.0,
        base: 0.0,
        color: [0.8, 0.8, 0.8],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    // No `set_particle_depth`: this is the scene-depth image, not the upload.
    draw(&mut life, &drawn, &camera);
    assert_eq!(
        life.device.particle_stats().alive,
        1,
        "the particle between the eye and the floor did not spawn"
    );
    settings.origin = [0.0, -0.4, 2.0];
    drawn.scene.particles = settings.clone();
    draw(&mut life, &drawn, &camera);
    settings.rate = 0.0;
    drawn.scene.particles = settings;
    draw(&mut life, &drawn, &camera);
    let gpu = life.device.particle_positions().expect("positions");
    assert_eq!(
        gpu.len(),
        1,
        "image depth retired the wrong set: {gpu:?}"
    );
    assert!(
        gpu[0][1] > 0.0,
        "the survivor is not the particle in front of the floor: {gpu:?}"
    );
}

#[test]
fn overlapping_flights_stay_valid_and_the_missing_barrier_does_not() {
    if let Ok(reason) = std::env::var("GENOS_FX_SYNC_SKIP") {
        assert!(
            !reason.is_empty(),
            "GENOS_FX_SYNC_SKIP needs the reason the validation layer cannot run"
        );
        eprintln!("particle sync test skipped: {reason}");
        return;
    }
    if std::env::var_os("GENOS_FX_SYNC_CHILD").is_none() {
        let clean = sync_child("clean");
        assert!(
            layer_loaded(&clean),
            "validation layer did not load:\n{clean}"
        );
        assert!(
            sync_log_clean(&clean),
            "clean overlapped flights reported a hazard or VUID:\n{clean}"
        );
        let skipped = sync_child("skip");
        assert!(
            layer_loaded(&skipped),
            "validation layer did not load for the barrier child:\n{skipped}"
        );
        assert!(
            skipped.contains("SYNC-HAZARD") && skipped.contains("vkCmdDispatch"),
            "removing only the compute barrier did not trip synchronization validation:\n{skipped}"
        );
        let reflected = sync_child("reflect");
        assert!(
            !reflected.contains("VUID-VkRenderPassBeginInfo-renderPass-00904"),
            "reflection pass is incompatible with the framebuffer:\n{reflected}"
        );
        assert!(
            !reflected.contains("SHADER_READ_ONLY_OPTIMAL"),
            "depth collision left the scene depth in shader-read layout:\n{reflected}"
        );
        let handles = particle_handles(&reflected);
        assert!(
            !handles.is_empty(),
            "reflect child printed no particle handles:\n{reflected}"
        );
        let faults = reflect_particle_faults(&reflected, &handles);
        assert!(
            faults.is_empty(),
            "reflection frames reported a particle hazard:\n{}\n{reflected}",
            faults.join("\n")
        );
        return;
    }
    match std::env::var("GENOS_FX_SYNC_CHILD").expect("child mode").as_str() {
        "skip" => {
            let (_w, _h, mut life) = open();
            let camera = Camera::new(0.0, 2.0, 0.0);
            life.device.suspend_lighting();
            life.device.set_particle_fault(ParticleFault::SkipFlightBarrier);
            overlap(&mut life, &emitter(8, 4), &camera, 4);
        }
        "reflect" => {
            let (_w, _h, mut life) = open();
            let camera = Camera::new(0.0, 2.0, -2.0);
            let mut settings = emitter(8, 4);
            settings.depth_collision = true;
            let mut drawn = world(settings);
            drawn.scene.reflections.enabled = true;
            drawn.reflectors.push(Reflector {
                origin: [0.0, 1.5, 1.0],
                normal: [0.0, 0.0, 1.0],
                tangent: [1.0, 0.0, 0.0],
                bitangent: [0.0, 1.0, 0.0],
                ..Reflector::default()
            });
            for _ in 0..4 {
                let _ = life.window.pump();
                life.device.set_particle_dt(DT);
                life.device
                    .draw_with_overlay(&drawn, &camera, &[], false, false)
                    .expect("reflect submit");
            }
            for handle in life.device.particle_validation_handles() {
                eprintln!("fx_particle_handle {handle:#x}");
            }
            life.device.finish_particles().expect("finish");
        }
        _ => {
            let (_w, _h, mut life) = open();
            let camera = Camera::new(0.0, 2.0, 0.0);
            life.device.suspend_lighting();
            overlap(&mut life, &emitter(8, 4), &camera, 4);
        }
    }
}

fn sync_child(mode: &str) -> String {
    let exe = std::env::current_exe().expect("test exe");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "overlapping_flights_stay_valid_and_the_missing_barrier_does_not",
            "--nocapture",
        ])
        .env("GENOS_FX_SYNC_CHILD", mode)
        .env("VK_INSTANCE_LAYERS", "VK_LAYER_KHRONOS_validation")
        .env(
            "VK_LAYER_ENABLES",
            "VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT",
        )
        .env("VK_KHRONOS_VALIDATION_VALIDATE_SYNC", "true")
        .env("VK_LOADER_DEBUG", "error,warn,layer")
        .output()
        .expect("sync child");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() && !text.contains("Validation Error") {
        use std::fmt::Write as _;
        let _ = write!(text, "\nchild exit status: {}", output.status);
    }
    text
}

fn layer_loaded(text: &str) -> bool {
    text.contains("Insert instance layer \"VK_LAYER_KHRONOS_validation\"")
}

fn particle_handles(text: &str) -> Vec<u64> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("fx_particle_handle ")?;
            let hex = rest.trim().trim_start_matches("0x");
            u64::from_str_radix(hex, 16).ok()
        })
        .collect()
}

fn handle_in_line(line: &str, handle: u64) -> bool {
    if handle == 0 {
        return false;
    }
    let needle = format!("{handle:#x}");
    let mut rest = line;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos + needle.len()..];
        let continues = after.chars().next().is_some_and(|ch| ch.is_ascii_hexdigit());
        if !continues {
            return true;
        }
        rest = &rest[pos + needle.len()..];
    }
    false
}

fn line_mentions_particle(line: &str, handles: &[u64]) -> bool {
    handles.iter().any(|handle| handle_in_line(line, *handle))
}

/// Lighting's fill read. Binding 0 against a fill is also the particle touch
/// witness, so the pipeline and buffer have to be outside the particle set.
fn lighting_fill_read_after_write(line: &str) -> bool {
    line.contains("SYNC-HAZARD-READ-AFTER-WRITE")
        && line.contains("vkCmdDispatch()")
        && line.contains("Hazard READ_AFTER_WRITE")
        && line.contains("VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, binding #0 index 0")
        && line.contains("usage: SYNC_COMPUTE_SHADER_SHADER_STORAGE_READ")
        && line.contains("prior_usage: SYNC_COPY_TRANSFER_WRITE")
        && line.contains("command: vkCmdFillBuffer")
}

/// Scene and reflection push ranges. A particle layout that hits the same VUID
/// is a different pipeline and stays a failure.
fn known_push_constant_range(line: &str) -> bool {
    if !line.contains("VUID-vkCmdPushConstants-offset-01796") {
        return false;
    }
    let overlap = "overlapping VkPushConstantRange stageFlags (VK_SHADER_STAGE_VERTEX_BIT|VK_SHADER_STAGE_FRAGMENT_BIT), offset (0), and size (80)";
    if !line.contains(overlap) {
        return false;
    }
    line.contains("stageFlags (VK_SHADER_STAGE_VERTEX_BIT, offset (0), and size (64)")
        || line.contains("stageFlags (VK_SHADER_STAGE_FRAGMENT_BIT, offset (64), and size (16)")
}

fn reflect_error_allowlisted(line: &str, handles: &[u64]) -> bool {
    if line_mentions_particle(line, handles) {
        return false;
    }
    lighting_fill_read_after_write(line) || known_push_constant_range(line)
}

fn reflect_particle_faults(text: &str, handles: &[u64]) -> Vec<String> {
    text.lines()
        .filter(|line| line.contains("Validation Error") || line.contains("SYNC-HAZARD"))
        .filter(|line| !reflect_error_allowlisted(line, handles))
        .filter(|line| line_mentions_particle(line, handles))
        .map(|line| line.trim().to_string())
        .collect()
}

fn lighting_raw(pipeline: u64, buffer: u64) -> String {
    format!(
        "Validation Error: [ SYNC-HAZARD-READ-AFTER-WRITE ] vkCmdDispatch():  Hazard READ_AFTER_WRITE for VkBuffer {buffer:#x}[] in VkCommandBuffer 0x7f[], VkPipeline {pipeline:#x}[], and VkDescriptorSet 0x1210000000121[], type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, binding #0 index 0. Access info (usage: SYNC_COMPUTE_SHADER_SHADER_STORAGE_READ, prior_usage: SYNC_COPY_TRANSFER_WRITE, write_barriers: SYNC_COMPUTE_SHADER_SHADER_STORAGE_WRITE, command: vkCmdFillBuffer, seq_no: 80, reset_no: 2)."
    )
}

fn push_range(stage: &str, layout: u64) -> String {
    format!(
        "Validation Error: [ VUID-vkCmdPushConstants-offset-01796 ] vkCmdPushConstants():  stageFlags ({stage},  must contain all stages in overlapping VkPushConstantRange stageFlags (VK_SHADER_STAGE_VERTEX_BIT|VK_SHADER_STAGE_FRAGMENT_BIT), offset (0), and size (80) in VkPipelineLayout {layout:#x}[]."
    )
}

fn hex(text: &str) -> u64 {
    u64::from_str_radix(text, 16).expect("hex handle")
}

fn assert_no_particle_fault(text: &str, handles: &[u64]) {
    assert_eq!(reflect_particle_faults(text, handles), Vec::<String>::new());
}

#[test]
fn the_reflect_allowlist_is_the_lighting_fill_and_the_push_range() {
    let particle = [hex("aa00000000aa"), hex("bb00000000bb"), hex("cc00000000cc")];
    let lighting = lighting_raw(hex("11f000000011f"), hex("1120000000112"));
    assert!(reflect_error_allowlisted(&lighting, &particle), "lighting fill read");
    assert_no_particle_fault(&lighting, &particle);

    let on_pipe = lighting_raw(hex("aa00000000aa"), hex("1120000000112"));
    assert!(!reflect_error_allowlisted(&on_pipe, &particle));
    assert_eq!(reflect_particle_faults(&on_pipe, &particle).len(), 1);

    let on_buffer = lighting_raw(hex("11f000000011f"), hex("bb00000000bb"));
    assert_eq!(reflect_particle_faults(&on_buffer, &particle).len(), 1);

    let other_binding = lighting.replace("binding #0 index 0", "binding #1 index 0");
    assert!(!reflect_error_allowlisted(&other_binding, &particle));
    assert_no_particle_fault(&other_binding, &particle);
    let other_binding_on_pipe = other_binding.replace("0x11f000000011f", "0xaa00000000aa");
    assert_eq!(reflect_particle_faults(&other_binding_on_pipe, &particle).len(), 1);

    let vert = push_range(
        "VK_SHADER_STAGE_VERTEX_BIT, offset (0), and size (64)",
        hex("12a000000012a"),
    );
    let frag = push_range(
        "VK_SHADER_STAGE_FRAGMENT_BIT, offset (64), and size (16)",
        hex("12a000000012a"),
    );
    assert!(reflect_error_allowlisted(&vert, &particle));
    assert!(reflect_error_allowlisted(&frag, &particle));
    assert_no_particle_fault(&format!("{vert}\n{frag}"), &particle);

    let on_layout = push_range(
        "VK_SHADER_STAGE_VERTEX_BIT, offset (0), and size (64)",
        hex("cc00000000cc"),
    );
    assert_eq!(reflect_particle_faults(&on_layout, &particle).len(), 1);

    let other_size = push_range(
        "VK_SHADER_STAGE_VERTEX_BIT, offset (0), and size (32)",
        hex("12a000000012a"),
    );
    assert!(!reflect_error_allowlisted(&other_size, &particle));

    let cache = "Validation Error: [ VUID-VkWriteDescriptorSet-descriptorType-00333 ] Object 0: handle = 0xa100000000a1, type = VK_OBJECT_TYPE_BUFFER";
    assert_no_particle_fault(cache, &particle);
    let cache_on_particle = "Validation Error: [ VUID-VkWriteDescriptorSet-descriptorType-00333 ] Object 0: handle = 0xbb00000000bb, type = VK_OBJECT_TYPE_BUFFER";
    assert_eq!(reflect_particle_faults(cache_on_particle, &particle).len(), 1);

    let prefix = "Validation Error: [ SYNC-HAZARD-WRITE-AFTER-READ ] VkBuffer 0xaa00000000a[]";
    assert_no_particle_fault(prefix, &particle);
    let exact = "Validation Error: [ SYNC-HAZARD-WRITE-AFTER-READ ] VkBuffer 0xaa00000000aa[]";
    assert_eq!(reflect_particle_faults(exact, &particle).len(), 1);
}

fn sync_log_clean(text: &str) -> bool {
    if text.contains("SYNC-HAZARD") {
        return false;
    }
    // Lavapipe's maxStorageBufferRange is 128MB. The lighting cache descriptor
    // is larger, so this VUID fires at device init even with lighting suspended.
    // It is the device-limit entry in lint/vk-device-limits.txt, not a particle hazard.
    text.lines().all(|line| {
        !line.contains("Validation Error")
            || line.contains("VUID-VkWriteDescriptorSet-descriptorType-00333")
    })
}

#[test]
fn lavapipe_cpu_frame_time_with_the_fence_and_without_it() {
    let (_w, _h, mut life) = open();
    let camera = Camera::new(0.0, 2.0, 0.0);
    let on = emitter(256, 64);
    let mut off = on.clone();
    off.enabled = false;
    // One dispatch creates the pass, so the old fence has something to wait.
    life.device.set_particle_dt(DT);
    life.device
        .draw(&world(on.clone()), &camera, true)
        .expect("warm")
        .expect("readback");
    let off_new = cpu_frame_ms(&mut life, &world(off.clone()), &camera, false);
    let off_old = cpu_frame_ms(&mut life, &world(off), &camera, true);
    let on_new = cpu_frame_ms(&mut life, &world(on.clone()), &camera, false);
    let on_old = cpu_frame_ms(&mut life, &world(on), &camera, true);
    life.device.set_particle_cpu_fence(false);
    eprintln!(
        "lavapipe cpu ms/frame emitter_off new={off_new:.3} old_fence={off_old:.3} emitter_on new={on_new:.3} old_fence={on_old:.3}"
    );
    assert!(off_new > 0.0 && on_new > 0.0);
}

fn cpu_frame_ms(
    life: &mut Lifecycle<Renderer>,
    world: &World,
    camera: &Camera,
    fence: bool,
) -> f64 {
    life.device.set_particle_cpu_fence(fence);
    life.device.set_particle_dt(DT);
    for _ in 0..2 {
        let _ = life.window.pump();
        life.device
            .draw_with_overlay(world, camera, &[], false, false)
            .expect("warmup");
    }
    let start = std::time::Instant::now();
    let frames = 8u32;
    for _ in 0..frames {
        let _ = life.window.pump();
        life.device
            .draw_with_overlay(world, camera, &[], false, false)
            .expect("timed submit");
    }
    life.device.finish_particles().expect("finish");
    start.elapsed().as_secs_f64() * 1.0e3 / f64::from(frames)
}
