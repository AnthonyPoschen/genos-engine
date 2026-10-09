//! GI v2 phase 1 gate: the stress building traced as triangles matches the analytic
//! trace within the reference's noise, at the five `reference_views.rhai` poses.
//!
//! CPU only (no window, no GPU). Heavy, so ignored by default:
//! `cargo test --release -p genos-stress --test mesh_reference -- --ignored --nocapture`

use genos_debug::reference::{render, RefSetup};
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
fn stress_views_traced_as_triangles_match_the_shapes() {
    // reference_views.rhai: noon, sun frozen, default lamps.
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
        (
            "doorway",
            [12.0, 1.6, 20.5],
            aim([12.0, 1.6, 20.5], [22.0, 1.2, 20.5]),
        ),
        (
            "outside",
            [12.5, 1.7, 34.0],
            aim([12.5, 1.7, 34.0], [12.5, 1.5, 22.0]),
        ),
        (
            "corner",
            [3.5, 1.6, 3.5],
            aim([3.5, 1.6, 3.5], [0.2, 0.4, 0.2]),
        ),
    ];
    let mut worst = 0.0f32;
    for (name, eye, (yaw, pitch)) in views {
        let setup = |triangles| RefSetup {
            scene: scene.clone(),
            eye,
            yaw,
            pitch,
            width: 160,
            height: 90,
            spp: 64,
            noise_target: 0.03,
            max_spp: 256,
            seconds: 60.0,
            max_bounces: 64,
            triangles,
        };
        let a = render(&setup(false));
        let b = render(&setup(true));
        let (mut diff, mut sum, mut hit_diff) = (0.0f64, 0.0f64, 0usize);
        for i in 0..a.linear.len() {
            if a.hit[i] != b.hit[i] {
                hit_diff += 1;
            } else if a.hit[i] {
                diff += (luma(a.linear[i]) - luma(b.linear[i])).abs();
                sum += luma(a.linear[i]);
            }
        }
        let mean =
            |r: &genos_debug::reference::Reference| r.linear.iter().map(|c| luma(*c)).sum::<f64>();
        let rel = (diff / sum.max(1e-9)) as f32;
        let bias = (mean(&b) / mean(&a) - 1.0) as f32;
        let noise = a.noise.max(b.noise);
        println!(
            "{name}: mean_rel_diff {rel:.4} bias {bias:+.4} noise {noise:.4} hit_diff {hit_diff} spp {}/{} {:.1}s/{:.1}s",
            a.spp, b.spp, a.seconds, b.seconds
        );
        // Both traces draw the same random numbers per pixel, so the difference is
        // the geometry alone (measured 2026-10-10: at most 0.03 %). The bound still
        // allows two independent traces (2x the noise plus 1 %).
        assert!(rel < 2.0 * noise + 0.01, "{name}: {rel} vs noise {noise}");
        assert!(bias.abs() < 0.01, "{name}: bias {bias}");
        assert!(
            hit_diff * 1000 < a.linear.len(),
            "{name}: coverage differs in {hit_diff} pixels"
        );
        worst = worst.max(rel);
    }
    println!("worst mean_rel_diff {worst:.4}");
}
