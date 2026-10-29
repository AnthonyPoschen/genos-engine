//! Emissive triangle lights against the analytic irradiance of a rectangle.
//!
//! Y is up. A ceiling panel sits in a plane of constant y and faces down.
//! The rectangle integral is the projected solid angle times radiance.

use genos_math::emissive::{
    irradiance_from_area_sample, parallel_rectangle_irradiance, solid_angle_pdf, ParallelRectangle,
    Triangle,
};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1.0e-30)
}

#[test]
fn emissive_factor_times_strength_is_radiance() {
    let le = genos_math::emissive::emitted_radiance([0.2, 0.5, 1.5], 4.0);
    assert!(close(le[0], 0.8, 1e-15));
    assert!(close(le[1], 2.0, 1e-15));
    assert!(close(le[2], 6.0, 1e-15));
    // An omitted `KHR_materials_emissive_strength` is strength 1.
    let plain = genos_math::emissive::emitted_radiance([0.2, 0.5, 1.5], 1.0);
    assert!(close(plain[0], 0.2, 1e-15));
    assert!(close(plain[1], 0.5, 1e-15));
    assert!(close(plain[2], 1.5, 1e-15));
}

#[test]
fn right_triangle_area_is_half_base_times_height() {
    // Base 2 along X, height 3 along Z, tilted so the normal is not an axis.
    let tri = Triangle {
        v0: [0.0, 0.0, 0.0],
        v1: [1.0, 1.0, 0.0],
        v2: [0.0, 1.0, 1.0],
        radiance: [1.0, 1.0, 1.0],
    };
    // |(1,1,0) × (0,1,1)| / 2 = |(1,-1,1)| / 2 = sqrt(3) / 2.
    let expected = 3.0_f64.sqrt() / 2.0;
    assert!(close(tri.area(), expected, 1e-12));
    let n = tri.normal();
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    assert!(close(len, 1.0, 1e-12));
    assert!(close(n[0], 1.0 / 3.0_f64.sqrt(), 1e-12));
    assert!(close(n[1], -1.0 / 3.0_f64.sqrt(), 1e-12));
    assert!(close(n[2], 1.0 / 3.0_f64.sqrt(), 1e-12));
}

#[test]
fn area_sample_uses_the_shading_barycentric_map() {
    let tri = ceiling_panel(0.0, 1.0, 0.0, 1.0, 1.0, [1.0, 1.0, 1.0])[0];
    let sample = tri.sample_area(0.25, 0.5);
    assert!(sample.is_some(), "a positive-area triangle has a sample");
    let Some(sample) = sample else {
        return;
    };
    // u = 0.25 → sqrt(u) = 0.5 → weights (0.5, 0.25, 0.25). Area of this
    // right triangle is 0.5, so the area pdf is 2.
    assert!(close(sample.pdf_area, 2.0, 1e-12));
    assert!(close(sample.point[0], 0.25, 1e-12));
    assert!(close(sample.point[1], 1.0, 1e-12));
    assert!(close(sample.point[2], 0.25, 1e-12));
    let weights = barycentric(tri, sample.point);
    assert!(weights.0 >= -1e-12 && weights.1 >= -1e-12 && weights.2 >= -1e-12);
    assert!(close(weights.0 + weights.1 + weights.2, 1.0, 1e-12));
    assert!(tri.sample_area(-0.1, 0.5).is_none());
    assert!(tri.sample_area(0.5, 1.1).is_none());
}

#[test]
fn solid_angle_pdf_is_area_pdf_times_distance_squared_over_cosine() {
    let origin = [0.0, 0.0, 0.0];
    let point = [0.0, 2.0, 0.0];
    let light_normal = [0.0, -1.0, 0.0];
    let pdf = solid_angle_pdf(origin, point, light_normal, 4.0);
    assert!(pdf.is_some());
    let Some(pdf) = pdf else {
        return;
    };
    // r² = 4, cos θ_light = 1, pdf_ω = 4 * 4 / 1.
    assert!(close(pdf, 16.0, 1e-12));
    let lit = irradiance_from_area_sample(
        origin,
        [0.0, 1.0, 0.0],
        light_normal,
        [2.0, 0.0, 0.0],
        point,
        4.0,
    );
    // Le * cos θ_receiver / pdf_ω = 2 * 1 / 16.
    assert!(close(lit[0], 0.125, 1e-12));
    assert!(close(lit[1], 0.0, 1e-12));
    assert!(close(lit[2], 0.0, 1e-12));
}

#[test]
fn back_face_grazing_and_coincident_samples_add_nothing() {
    let origin = [0.0, 0.0, 0.0];
    let point = [0.0, 2.0, 0.0];
    assert!(solid_angle_pdf(origin, point, [0.0, 1.0, 0.0], 1.0).is_none());
    assert!(solid_angle_pdf(origin, point, [1.0, 0.0, 0.0], 1.0).is_none());
    assert!(solid_angle_pdf(origin, origin, [0.0, -1.0, 0.0], 1.0).is_none());
    let away = irradiance_from_area_sample(
        origin,
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [3.0, 3.0, 3.0],
        point,
        1.0,
    );
    assert!(away.iter().all(|c| *c == 0.0 && c.is_finite()));
    let same = irradiance_from_area_sample(
        origin,
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [3.0, 3.0, 3.0],
        origin,
        1.0,
    );
    assert!(same.iter().all(|c| *c == 0.0 && c.is_finite()));
    let degenerate = Triangle {
        v0: [0.0, 1.0, 0.0],
        v1: [1.0, 1.0, 0.0],
        v2: [2.0, 1.0, 0.0],
        radiance: [1.0, 1.0, 1.0],
    };
    assert!(close(degenerate.area(), 0.0, 0.0));
    assert!(degenerate.sample_area(0.3, 0.3).is_none());
}

#[test]
fn parallel_rectangle_matches_a_quadrature_and_the_hemisphere() {
    let rect = ParallelRectangle {
        height: 1.25,
        min_x: -0.2,
        max_x: 1.4,
        min_y: -0.5,
        max_y: 0.7,
        radiance: [1.0, 1.0, 1.0],
    };
    let analytic = parallel_rectangle_irradiance(rect);
    let quad = quadrature(rect, 96);
    assert!(
        rel(analytic[0], quad) < 1.0e-4,
        "analytic {} quadrature {quad}",
        analytic[0]
    );
    // An infinite parallel plane is the hemisphere: projected solid angle π.
    let sky = parallel_rectangle_irradiance(ParallelRectangle {
        height: 1.0,
        min_x: -1.0e6,
        max_x: 1.0e6,
        min_y: -1.0e6,
        max_y: 1.0e6,
        radiance: [1.0, 0.0, 0.0],
    });
    assert!(rel(sky[0], std::f64::consts::PI) < 1.0e-9, "sky {}", sky[0]);
    assert!(close(sky[1], 0.0, 0.0));
    let behind = parallel_rectangle_irradiance(ParallelRectangle {
        height: 0.0,
        ..rect
    });
    assert!(behind.iter().all(|c| *c == 0.0));
}

#[test]
fn stratified_area_samples_converge_to_rectangle_irradiance() {
    let le = genos_math::emissive::emitted_radiance([0.5, 0.25, 1.0], 3.0);
    let height = 1.25;
    let min_x = -0.2;
    let max_x = 1.4;
    let min_z = -0.5;
    let max_z = 0.7;
    let analytic = parallel_rectangle_irradiance(ParallelRectangle {
        height,
        min_x,
        max_x,
        min_y: min_z,
        max_y: max_z,
        radiance: le,
    });
    let tris = ceiling_panel(min_x, max_x, min_z, max_z, height, le);
    let mut errors = [0.0; 4];
    for (slot, cells) in [4u32, 8, 16, 32].into_iter().enumerate() {
        let mean = stratified_irradiance(&tris, cells);
        // Grade the green channel so a swapped factor fails on its own.
        errors[slot] = rel(mean[1], analytic[1]);
        assert!(mean.iter().all(|c| c.is_finite()));
        for channel in 0..3 {
            assert!(
                rel(mean[channel], analytic[channel]) < 0.02,
                "cells {cells} channel {channel} mean {} analytic {}",
                mean[channel],
                analytic[channel]
            );
        }
    }
    assert!(
        errors[0] > errors[1] && errors[1] > errors[2] && errors[2] > errors[3],
        "relative errors did not fall: {errors:?}"
    );
    assert!(
        errors[3] < 5.0e-4,
        "32² per triangle should be under 5e-4, was {}",
        errors[3]
    );
}

fn ceiling_panel(
    min_x: f64,
    max_x: f64,
    min_z: f64,
    max_z: f64,
    height: f64,
    radiance: [f64; 3],
) -> [Triangle; 2] {
    let v00 = [min_x, height, min_z];
    let v10 = [max_x, height, min_z];
    let v11 = [max_x, height, max_z];
    let v01 = [min_x, height, max_z];
    [
        Triangle {
            v0: v00,
            v1: v10,
            v2: v01,
            radiance,
        },
        Triangle {
            v0: v10,
            v1: v11,
            v2: v01,
            radiance,
        },
    ]
}

fn barycentric(tri: Triangle, point: [f64; 3]) -> (f64, f64, f64) {
    let whole = signed_area(tri.v0, tri.v1, tri.v2);
    let b0 = signed_area(point, tri.v1, tri.v2) / whole;
    let b1 = signed_area(tri.v0, point, tri.v2) / whole;
    let b2 = signed_area(tri.v0, tri.v1, point) / whole;
    (b0, b1, b2)
}

fn signed_area(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let ux = b[0] - a[0];
    let uy = b[1] - a[1];
    let uz = b[2] - a[2];
    let vx = c[0] - a[0];
    let vy = c[1] - a[1];
    let vz = c[2] - a[2];
    let cx = uy * vz - uz * vy;
    let cy = uz * vx - ux * vz;
    let cz = ux * vy - uy * vx;
    0.5 * (cx * cx + cy * cy + cz * cz).sqrt().copysign(cy)
}

fn stratified_irradiance(tris: &[Triangle; 2], cells: u32) -> [f64; 3] {
    let area0 = tris[0].area();
    let area1 = tris[1].area();
    let total = area0 + area1;
    let mut sum = [0.0; 3];
    let mut drawn = 0u32;
    let n = f64::from(cells);
    for tri in tris {
        let pick = tri.area() / total;
        for i in 0..cells {
            for j in 0..cells {
                let u = (f64::from(i) + 0.5) / n;
                let v = (f64::from(j) + 0.5) / n;
                let Some(sample) = tri.sample_area(u, v) else {
                    continue;
                };
                let pdf_area = pick * sample.pdf_area;
                let lit = irradiance_from_area_sample(
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    tri.normal(),
                    tri.radiance,
                    sample.point,
                    pdf_area,
                );
                for channel in 0..3 {
                    sum[channel] += lit[channel];
                }
                drawn += 1;
            }
        }
    }
    let count = f64::from(drawn.max(1));
    assert!(drawn == cells * cells * 2, "missed samples {drawn}");
    sum.map(|c| c / count)
}

fn quadrature(rect: ParallelRectangle, cells: u32) -> f64 {
    let dx = (rect.max_x - rect.min_x) / f64::from(cells);
    let dy = (rect.max_y - rect.min_y) / f64::from(cells);
    let mut sum = 0.0;
    for i in 0..cells {
        let x = rect.min_x + (f64::from(i) + 0.5) * dx;
        for j in 0..cells {
            let y = rect.min_y + (f64::from(j) + 0.5) * dy;
            let r2 = x * x + y * y + rect.height * rect.height;
            sum += (rect.height * rect.height) / (r2 * r2);
        }
    }
    sum * dx * dy
}
