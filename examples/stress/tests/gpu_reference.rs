//! The GPU reference tracer (genos-gpuref) against the CPU proof: at the five
//! `reference_views.rhai` poses of the stress building (noon sun, lamps, sky), the
//! GPU trace run far past the CPU's noise must differ from the CPU trace by no more
//! than the CPU trace's own noise, with no overall bias.
//!
//! Needs a GPU with VK_KHR_ray_query, and runs for minutes, so ignored by default:
//! `cargo test --release -p genos-stress --test gpu_reference -- --ignored --nocapture`
//! (GENOS_GPUREF_OUT=dir saves cpu-*.png and gpu-*.png).

use genos_debug::reference::{render, render_gpu, RefSetup};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};

fn luma(c: [f32; 3]) -> f64 {
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]) as f64
}

fn aim(from: [f32; 3], to: [f32; 3]) -> (f32, f32) {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (d[0].atan2(-d[2]), (d[1] / len).asin())
}

#[test]
#[ignore]
fn gpu_reference_matches_the_cpu_proof() {
    let stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 50,
            layout: Layout::Spread,
        },
        1.0,
        0.25,
        120.0,
    );
    let scene = stage.world.scene.clone();
    let views = [
        ("hall", [14.5, 1.7, 23.5], (-0.3554, 0.08)),
        ("room-a", [7.2, 1.7, 0.9], (-2.556, -0.12)),
        ("doorway", [12.0, 1.6, 20.5], aim([12.0, 1.6, 20.5], [22.0, 1.2, 20.5])),
        ("outside", [12.5, 1.7, 34.0], aim([12.5, 1.7, 34.0], [12.5, 1.5, 22.0])),
        ("corner", [3.5, 1.6, 3.5], aim([3.5, 1.6, 3.5], [0.2, 0.4, 0.2])),
    ];
    let out = std::env::var("GENOS_GPUREF_OUT").ok();
    let cpu_triangles = std::env::var("GENOS_GPUREF_CPU_TRIANGLES").is_ok_and(|v| v == "1");
    let (mut cpu_s, mut gpu_s) = (0.0f32, 0.0f32);
    let only = std::env::var("GENOS_GPUREF_VIEWS").ok();
    let cpu_max: u32 = std::env::var("GENOS_GPUREF_CPU_SPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
    for (name, eye, (yaw, pitch)) in views {
        if only.as_ref().is_some_and(|o| !o.split(',').any(|v| v == name)) {
            continue;
        }
        let setup = |spp, noise_target, max_spp, triangles| RefSetup {
            scene: scene.clone(),
            eye,
            yaw,
            pitch,
            width: 160,
            height: 90,
            spp,
            noise_target,
            max_spp,
            seconds: 600.0,
            max_bounces: 64,
            triangles,
        };
        let cpu = render(&setup(64, 0.0, cpu_max, cpu_triangles));
        let gpu = render_gpu(&setup(16, cpu.noise / 8.0, 1 << 22, true)).expect("gpu reference");
        cpu_s += cpu.seconds;
        gpu_s += gpu.seconds;
        let (mut diff, mut sum, mut hit_diff, mut sum_g) = (0.0f64, 0.0f64, 0usize, 0.0f64);
        for i in 0..cpu.linear.len() {
            if cpu.hit[i] != gpu.hit[i] {
                hit_diff += 1;
            } else if cpu.hit[i] {
                diff += (luma(cpu.linear[i]) - luma(gpu.linear[i])).abs();
                sum += luma(cpu.linear[i]);
                sum_g += luma(gpu.linear[i]);
            }
        }
        let rel = (diff / sum.max(1e-9)) as f32;
        let bias = (sum_g / sum.max(1e-9) - 1.0) as f32;
        // Per-pixel paths are throughput: cpu rate over gpu rate at equal paths.
        let speed = (gpu.spp as f32 / gpu.seconds) / (cpu.spp as f32 / cpu.seconds);
        println!(
            "{name}: rel diff {rel:.4} bias {bias:+.4} | cpu noise {:.4} spp {} {:.1}s | gpu noise {:.4} spp {} {:.1}s | paths/s x{speed:.0} | hit_diff {hit_diff}",
            cpu.noise, cpu.spp, cpu.seconds, gpu.noise, gpu.spp, gpu.seconds
        );
        if let Some(dir) = &out {
            let _ = std::fs::create_dir_all(dir);
            cpu.image.save(&std::path::Path::new(dir).join(format!("cpu-{name}.png"))).ok();
            gpu.image.save(&std::path::Path::new(dir).join(format!("gpu-{name}.png"))).ok();
        }
        // Independent traces: the mean |difference| of two estimates whose noises
        // are n_cpu and n_gpu << n_cpu is about sqrt(2 / pi) n_cpu; allow 1.5 n_cpu.
        assert!(rel < 1.5 * cpu.noise + 0.002, "{name}: diff {rel} vs cpu noise {}", cpu.noise);
        assert!(bias.abs() < 0.01, "{name}: bias {bias}");
        assert!(hit_diff * 1000 < cpu.linear.len(), "{name}: coverage differs in {hit_diff} pixels");
    }
    println!("total cpu {cpu_s:.1}s gpu {gpu_s:.1}s");
}
