//! Closed-form fixtures. The good path is an independent calculation.
//! The broken path is a deliberate mistake.

use genos_lighting_ref::{blur_relative, MetricParams, Picture};

use crate::report::{judge, Metric};
use crate::Fixture;

const BEER_LAMBERT_MAX_ABS: f64 = 1.0e-4;
const HEIGHT_FOG_DENSITY_MAX_ABS: f64 = 1.0e-4;
const PHASE_HG_MAX_ABS: f64 = 1.0e-6;
const HOMOGENEOUS_INSCATTER_MAX_ABS: f64 = 1.0e-3;
const IMAGE_GRADE_BLUR_REL: f64 = 0.12;

/// `(g, cos θ, phase)`. Literals are locked from an independent evaluation.
const HG_SAMPLES: [(f64, f64, f64); 7] = [
    (0.0, 1.0, 0.079_577_471_545_947_67),
    (0.0, 0.0, 0.079_577_471_545_947_67),
    (0.0, -1.0, 0.079_577_471_545_947_67),
    (0.4, 1.0, 0.309_467_944_900_907_5),
    (0.4, 0.0, 0.053_503_530_222_411_88),
    (0.4, -1.0, 0.024_360_450_473_249_286),
    (0.7, 0.5, 0.057_798_913_563_976_75),
];

const BEER_DISTANCES: [f64; 6] = [0.0, 0.5, 1.0, 2.0, 4.0, 8.0];
const HEIGHTS: [f64; 6] = [0.0, 10.0, 20.0, 40.0, 80.0, 120.0];
const INSCATTER_DISTANCES: [f64; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];

pub(crate) fn evaluate_analytic(fixture: Fixture) -> Vec<Metric> {
    vec![
        graded_metric(
            "beer_lambert_max_abs",
            beer_lambert_max_abs(fixture),
            BEER_LAMBERT_MAX_ABS,
        ),
        graded_metric(
            "height_fog_density_max_abs",
            height_fog_density_max_abs(fixture),
            HEIGHT_FOG_DENSITY_MAX_ABS,
        ),
        graded_metric(
            "phase_hg_max_abs",
            phase_hg_max_abs(fixture),
            PHASE_HG_MAX_ABS,
        ),
        graded_metric(
            "homogeneous_inscatter_max_abs",
            homogeneous_inscatter_max_abs(fixture),
            HOMOGENEOUS_INSCATTER_MAX_ABS,
        ),
        image_grade(fixture),
    ]
}

fn graded_metric(name: &'static str, value: f64, threshold: f64) -> Metric {
    Metric {
        name,
        value: Some(value),
        threshold,
        status: judge(true, Some(value), threshold),
        note: "",
    }
}

fn beer_lambert_max_abs(fixture: Fixture) -> f64 {
    const SIGMA: f64 = 0.5;
    max_abs(BEER_DISTANCES, |distance| {
        let reference = (-SIGMA * distance).exp();
        let candidate = match fixture {
            Fixture::Good => transmittance_stepped(SIGMA, distance),
            Fixture::Broken => (1.0 - SIGMA * distance).max(0.0),
        };
        (candidate - reference).abs()
    })
}

/// Product of per-step transmittance. For constant extinction this is the
/// closed form up to rounding.
fn transmittance_stepped(sigma: f64, distance: f64) -> f64 {
    const STEPS: u32 = 64;
    if distance <= 0.0 {
        return 1.0;
    }
    let step = (-sigma * distance / f64::from(STEPS)).exp();
    let mut transmittance = 1.0;
    let mut index = 0u32;
    while index < STEPS {
        transmittance *= step;
        index += 1;
    }
    transmittance
}

fn height_fog_density_max_abs(fixture: Fixture) -> f64 {
    const SIGMA0: f64 = 0.08;
    const THICKNESS: f64 = 40.0;
    max_abs(HEIGHTS, |height| {
        let reference = SIGMA0 * (-height / THICKNESS).exp();
        let candidate = match fixture {
            Fixture::Good => height_euler(SIGMA0, THICKNESS, height),
            Fixture::Broken => SIGMA0 * (1.0 - height / THICKNESS).max(0.0),
        };
        (candidate - reference).abs()
    })
}

/// Forward Euler on `dσ/dy = -σ/H`. Small steps stay inside the threshold.
fn height_euler(sigma0: f64, thickness: f64, height: f64) -> f64 {
    const DY: f64 = 0.001;
    if height <= 0.0 {
        return sigma0;
    }
    let mut sigma = sigma0;
    let mut y = 0.0;
    while y < height {
        let step = DY.min(height - y);
        sigma *= 1.0 - step / thickness;
        y += step;
    }
    sigma
}

fn phase_hg_max_abs(fixture: Fixture) -> f64 {
    max_abs(HG_SAMPLES, |(g, mu, expected)| {
        let candidate = match fixture {
            Fixture::Good => henyey_greenstein(g, mu),
            Fixture::Broken => 1.0,
        };
        (candidate - expected).abs()
    })
}

/// Henyey-Greenstein. The integral over the sphere is 1.
fn henyey_greenstein(g: f64, mu: f64) -> f64 {
    let denom = 1.0 + g * g - 2.0 * g * mu;
    (1.0 - g * g) / (4.0 * std::f64::consts::PI * denom.powf(1.5))
}

fn homogeneous_inscatter_max_abs(fixture: Fixture) -> f64 {
    const SIGMA: f64 = 0.2;
    const SOURCE: f64 = 0.15;
    max_abs(INSCATTER_DISTANCES, |distance| {
        let reference = (SOURCE / SIGMA) * (1.0 - (-SIGMA * distance).exp());
        let candidate = match fixture {
            Fixture::Good => inscatter_exact_steps(SIGMA, SOURCE, distance),
            Fixture::Broken => SOURCE * distance,
        };
        (candidate - reference).abs()
    })
}

/// Piecewise exact integral for constant extinction and a constant source.
fn inscatter_exact_steps(sigma: f64, source: f64, distance: f64) -> f64 {
    const STEPS: u32 = 32;
    if distance <= 0.0 {
        return 0.0;
    }
    let ds = distance / f64::from(STEPS);
    let decay = (-sigma * ds).exp();
    let absorb = 1.0 - decay;
    let mut transmittance = 1.0;
    let mut sum = 0.0;
    let mut index = 0u32;
    while index < STEPS {
        sum += transmittance * (source / sigma) * absorb;
        transmittance *= decay;
        index += 1;
    }
    sum
}

fn image_grade(fixture: Fixture) -> Metric {
    let gain = match fixture {
        Fixture::Good => 1.0,
        Fixture::Broken => 1.4,
    };
    let graded = fog_picture(1.0).and_then(|reference| {
        let candidate = fog_picture(gain)?;
        blur_relative(&reference, &candidate, &MetricParams::default())
    });
    match graded {
        Ok(value) => graded_metric("image_grade_blur_rel", value, IMAGE_GRADE_BLUR_REL),
        Err(_) => Metric {
            name: "image_grade_blur_rel",
            value: None,
            threshold: IMAGE_GRADE_BLUR_REL,
            status: crate::report::Status::Fail,
            note: "picture grade failed to build",
        },
    }
}

fn fog_picture(gain: f32) -> Result<Picture, String> {
    const COL_DISTANCE: [f32; 8] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];
    let mut linear = Vec::with_capacity(COL_DISTANCE.len().saturating_mul(2).saturating_mul(3));
    for _row in 0..2 {
        for &distance in &COL_DISTANCE {
            let radiance = homogeneous_pixel(distance) * gain;
            linear.push(radiance);
            linear.push(radiance * 0.85);
            linear.push(radiance * 0.55);
        }
    }
    Picture::new(8, 2, linear)
}

fn homogeneous_pixel(distance: f32) -> f32 {
    let sigma = 0.2_f32;
    let source = 0.15_f32;
    let background = 0.25_f32;
    let transmittance = (-sigma * distance).exp();
    background * transmittance + (source / sigma) * (1.0 - transmittance)
}

fn max_abs<T: Copy, const N: usize>(samples: [T; N], error: impl Fn(T) -> f64) -> f64 {
    let mut max_abs = 0.0_f64;
    for sample in samples {
        max_abs = max_abs.max(error(sample));
    }
    max_abs
}
