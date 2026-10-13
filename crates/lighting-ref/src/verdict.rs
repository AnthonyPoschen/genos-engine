//! Pass, fail, or needs a person.
//!
//! A value inside the band around a threshold does not auto-pass. A metric
//! that improved and a metric that got worse, neither of them a clear fail,
//! also needs a person. One sample per pixel does not prove a pass: that
//! trace has no noise estimate. With a noise estimate, raw per-pixel error
//! inside that noise is not a fail. The blurred error and the luminance
//! ratio are the gates.

use std::collections::BTreeMap;

use crate::config::Config;
use crate::metrics::FrameMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricId {
    MeanRel,
    P99Rel,
    MeanRelDark,
    FlipMean,
    FlipP99,
    Flicker,
    Converge1,
    Converge2,
    Converge4,
    Converge8,
    UnfilledShare,
    BlurRel,
    LumaDelta,
}

impl MetricId {
    pub fn all() -> &'static [Self] {
        &[
            Self::MeanRel,
            Self::P99Rel,
            Self::MeanRelDark,
            Self::FlipMean,
            Self::FlipP99,
            Self::Flicker,
            Self::Converge1,
            Self::Converge2,
            Self::Converge4,
            Self::Converge8,
            Self::UnfilledShare,
            Self::BlurRel,
            Self::LumaDelta,
        ]
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::MeanRel => "mean_rel",
            Self::P99Rel => "p99_rel",
            Self::MeanRelDark => "mean_rel_dark",
            Self::FlipMean => "flip_mean",
            Self::FlipP99 => "flip_p99",
            Self::Flicker => "flicker",
            Self::Converge1 => "converge_1",
            Self::Converge2 => "converge_2",
            Self::Converge4 => "converge_4",
            Self::Converge8 => "converge_8",
            Self::UnfilledShare => "unfilled_share",
            Self::BlurRel => "blur_rel",
            Self::LumaDelta => "luma_delta",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MeanRel => "mean relative luminance",
            Self::P99Rel => "p99 relative luminance",
            Self::MeanRelDark => "dark-region relative luminance",
            Self::FlipMean => "FLIP mean",
            Self::FlipP99 => "FLIP p99",
            Self::Flicker => "still-camera flicker",
            Self::Converge1 => "error 1 frame after the event",
            Self::Converge2 => "error 2 frames after the event",
            Self::Converge4 => "error 4 frames after the event",
            Self::Converge8 => "error 8 frames after the event",
            Self::UnfilledShare => "unfilled-pixel share",
            Self::BlurRel => "blurred relative luminance",
            Self::LumaDelta => "mean luminance ratio error",
        }
    }

    /// Raw per-pixel scores. A value inside the reference noise is not a fail.
    /// The blurred error and the luminance ratio are not in this set: they are
    /// the gates, and reference noise does not waive them.
    pub fn uses_noise(self) -> bool {
        matches!(
            self,
            Self::MeanRel | Self::P99Rel | Self::MeanRelDark | Self::FlipMean | Self::FlipP99
        )
    }

    /// p99 tails sit wider than the mean noise.
    pub fn noise_is_a_tail(self) -> bool {
        matches!(self, Self::P99Rel | Self::FlipP99)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    NeedsHuman,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NeedsHuman => "needs human",
        }
    }

    fn worse(self, other: Self) -> Self {
        match (self, other) {
            (Self::Fail, _) | (_, Self::Fail) => Self::Fail,
            (Self::NeedsHuman, _) | (_, Self::NeedsHuman) => Self::NeedsHuman,
            _ => Self::Pass,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricRow {
    pub id: MetricId,
    pub value: Option<f64>,
    pub baseline: Option<f64>,
    pub verdict: Verdict,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub rows: Vec<MetricRow>,
    pub reasons: Vec<String>,
}

/// Values saved for a commit so a later run can compare.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Baseline {
    pub values: BTreeMap<String, f64>,
}

impl Baseline {
    pub fn from_metrics(metrics: &FrameMetrics) -> Self {
        let mut values = BTreeMap::new();
        for id in MetricId::all() {
            if let Some(value) = metric_value(metrics, *id) {
                values.insert(id.key().to_string(), value);
            }
        }
        Self { values }
    }

    pub fn get(&self, id: MetricId) -> Option<f64> {
        self.values.get(id.key()).copied()
    }
}

pub struct Judge<'a> {
    pub metrics: &'a FrameMetrics,
    pub config: &'a Config,
    pub baseline: Option<&'a Baseline>,
    pub reference_spp: u32,
    pub reference_noise: f64,
    /// Metrics that must have been measured. A still camera requires flicker.
    /// An event requires the four convergence samples.
    pub require: &'a [MetricId],
}

pub fn judge(input: Judge<'_>) -> Judgement {
    let mut rows = Vec::new();
    let mut reasons = Vec::new();
    for id in MetricId::all() {
        let row = judge_one(*id, &input);
        if row.verdict != Verdict::Pass {
            reasons.push(row.reason.clone());
        }
        rows.push(row);
    }
    let mut verdict = rows
        .iter()
        .fold(Verdict::Pass, |acc, row| acc.worse(row.verdict));
    if let Some(text) = disagreement(&rows, input.config.disagree_delta) {
        if verdict == Verdict::Pass {
            verdict = Verdict::NeedsHuman;
        }
        reasons.push(text);
    }
    if verdict == Verdict::Pass {
        reasons.push("every gated metric is under its band".to_string());
    }
    Judgement {
        verdict,
        rows,
        reasons,
    }
}

fn judge_one(id: MetricId, input: &Judge<'_>) -> MetricRow {
    let value = metric_value(input.metrics, id);
    let baseline = input.baseline.and_then(|b| b.get(id));
    let Some(value) = value else {
        let required = input.require.contains(&id);
        return MetricRow {
            id,
            value: None,
            baseline,
            verdict: if required {
                Verdict::NeedsHuman
            } else {
                Verdict::Pass
            },
            reason: if required {
                format!("{} was required and was not measured", id.label())
            } else {
                format!(
                    "{} was not measured; this checkpoint does not gate it",
                    id.label()
                )
            },
        };
    };
    let fail_at = input.config.fail_of(id.key());
    let band = input.config.human_band;
    let lo = fail_at * (1.0 - band);
    let hi = fail_at * (1.0 + band);
    let mut verdict = band_verdict(value, lo, hi);
    let mut reason = band_reason(id, value, fail_at, lo, hi, verdict);

    let mut waived = false;
    if id.uses_noise() {
        if input.reference_spp < 2 {
            if verdict != Verdict::Fail {
                verdict = Verdict::NeedsHuman;
                reason = format!(
                    "the reference has {} sample per pixel, so its noise is unknown; {} is {:.4}, not a pass",
                    input.reference_spp,
                    id.label(),
                    value
                );
            }
        } else {
            let ceiling = noise_ceiling(id, input);
            if ceiling > 0.0 && value <= ceiling {
                waived = true;
                verdict = Verdict::Pass;
                reason = format!(
                    "{label} is {value:.4}, within reference noise {noise:.4} (ceiling {ceiling:.4}); blurred error and the luminance ratio are the gate",
                    label = id.label(),
                    noise = input.reference_noise,
                );
            }
        }
    }

    if let Some(before) = baseline {
        if !waived {
            let delta = value - before;
            let limit = input.config.regress_of(id.key());
            let rlo = limit * (1.0 - band);
            let rhi = limit * (1.0 + band);
            let regress = if delta > rhi {
                Verdict::Fail
            } else if delta > rlo {
                Verdict::NeedsHuman
            } else {
                Verdict::Pass
            };
            if regress != Verdict::Pass {
                let regress_reason = format!(
                "{label} moved by {delta:+.4} from the baseline {before:.4} (regression line {limit:.4}, band {rlo:.4}..{rhi:.4})",
                label = id.label(),
            );
                verdict = verdict.worse(regress);
                if reason.is_empty() {
                    reason = regress_reason;
                } else if regress == Verdict::Fail || verdict == regress {
                    reason = format!("{reason} {regress_reason}");
                }
            }
        }
    }

    MetricRow {
        id,
        value: Some(value),
        baseline,
        verdict,
        reason,
    }
}

fn band_verdict(value: f64, lo: f64, hi: f64) -> Verdict {
    if value > hi {
        Verdict::Fail
    } else if value > lo {
        Verdict::NeedsHuman
    } else {
        Verdict::Pass
    }
}

fn band_reason(
    id: MetricId,
    value: f64,
    fail_at: f64,
    lo: f64,
    hi: f64,
    verdict: Verdict,
) -> String {
    match verdict {
        Verdict::Fail => format!(
            "{label} is {value:.4}, past {hi:.4} (fail {fail_at:.4})",
            label = id.label()
        ),
        Verdict::NeedsHuman => format!(
            "{label} is {value:.4}, inside the band around {fail_at:.4} ({lo:.4}..{hi:.4}); needs a person",
            label = id.label()
        ),
        Verdict::Pass => format!(
            "{label} is {value:.4}, under {lo:.4}",
            label = id.label()
        ),
    }
}

fn noise_ceiling(id: MetricId, input: &Judge<'_>) -> f64 {
    let noise = input.reference_noise.max(0.0);
    if noise == 0.0 || !id.uses_noise() {
        return 0.0;
    }
    let ceiling = noise * input.config.noise_multiple;
    if id.noise_is_a_tail() {
        ceiling * input.config.noise_tail
    } else {
        ceiling
    }
}

fn disagreement(rows: &[MetricRow], delta_min: f64) -> Option<String> {
    let mut improved: Vec<&MetricRow> = Vec::new();
    let mut regressed: Vec<&MetricRow> = Vec::new();
    for row in rows {
        let (Some(value), Some(before)) = (row.value, row.baseline) else {
            continue;
        };
        let delta = value - before;
        if delta < -delta_min {
            improved.push(row);
        } else if delta > delta_min {
            regressed.push(row);
        }
    }
    if improved.is_empty() || regressed.is_empty() {
        return None;
    }
    Some(format!(
        "{} improved and {} got worse; the metrics disagree",
        improved[0].id.label(),
        regressed[0].id.label()
    ))
}

pub fn metric_value(metrics: &FrameMetrics, id: MetricId) -> Option<f64> {
    match id {
        MetricId::MeanRel => metrics.mean_rel,
        MetricId::P99Rel => metrics.p99_rel,
        MetricId::MeanRelDark => metrics.mean_rel_dark,
        MetricId::FlipMean => metrics.flip_mean,
        MetricId::FlipP99 => metrics.flip_p99,
        MetricId::Flicker => metrics.flicker,
        MetricId::Converge1 => metrics.converge_1,
        MetricId::Converge2 => metrics.converge_2,
        MetricId::Converge4 => metrics.converge_4,
        MetricId::Converge8 => metrics.converge_8,
        MetricId::UnfilledShare => metrics.unfilled_share(),
        MetricId::BlurRel => metrics.blur_rel,
        MetricId::LumaDelta => metrics.luma_delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricParams;
    use crate::picture::Picture;

    fn quiet() -> FrameMetrics {
        let picture = Picture::solid(8, 4, [0.4, 0.4, 0.4]);
        crate::metrics::measure(
            &picture,
            &picture,
            Some(&[picture.clone(), picture.clone()]),
            &[],
            &MetricParams::default(),
        )
        .unwrap()
    }

    fn judged(
        metrics: &FrameMetrics,
        baseline: Option<&Baseline>,
        spp: u32,
        noise: f64,
    ) -> Judgement {
        judge(Judge {
            metrics,
            config: &Config::builtin(),
            baseline,
            reference_spp: spp,
            reference_noise: noise,
            require: &[],
        })
    }

    #[test]
    fn a_clean_picture_passes() {
        let metrics = quiet();
        let result = judged(&metrics, None, 16, 0.0);
        assert_eq!(result.verdict, Verdict::Pass, "{:?}", result.reasons);
    }

    #[test]
    fn a_large_bias_fails_and_a_near_bias_needs_a_person() {
        let reference = Picture::solid(8, 4, [0.4, 0.4, 0.4]);
        let hard = reference.scaled(1.4);
        let hard_m =
            crate::metrics::measure(&reference, &hard, None, &[], &MetricParams::default())
                .unwrap();
        let hard_j = judged(&hard_m, None, 16, 0.0);
        assert_eq!(hard_j.verdict, Verdict::Fail);
        assert!(hard_j
            .reasons
            .iter()
            .any(|r| r.contains("mean relative luminance")));

        let near = reference.scaled(1.075);
        let near_m =
            crate::metrics::measure(&reference, &near, None, &[], &MetricParams::default())
                .unwrap();
        let near_j = judged(&near_m, None, 16, 0.0);
        assert_eq!(near_j.verdict, Verdict::NeedsHuman, "{:?}", near_j.reasons);
        assert!(near_j.reasons.iter().any(|r| r.contains("needs a person")));
    }

    #[test]
    fn one_sample_does_not_auto_pass_and_a_huge_error_still_fails() {
        let metrics = quiet();
        let shy = judged(&metrics, None, 1, 0.0);
        assert_eq!(shy.verdict, Verdict::NeedsHuman, "{:?}", shy.reasons);
        assert!(shy.reasons.iter().any(|r| r.contains("noise is unknown")));

        let reference = Picture::solid(8, 4, [0.4, 0.4, 0.4]);
        let bad = reference.scaled(1.8);
        let metrics =
            crate::metrics::measure(&reference, &bad, None, &[], &MetricParams::default()).unwrap();
        assert_eq!(judged(&metrics, None, 1, 0.0).verdict, Verdict::Fail);
    }

    #[test]
    fn disagreement_without_a_hard_fail_needs_a_person() {
        let mut metrics = quiet();
        metrics.mean_rel = Some(0.03);
        metrics.flip_mean = Some(0.12);
        let mut baseline = Baseline::default();
        baseline.values.insert("mean_rel".into(), 0.05);
        baseline.values.insert("flip_mean".into(), 0.10);
        let result = judged(&metrics, Some(&baseline), 16, 0.0);
        assert_eq!(result.verdict, Verdict::NeedsHuman, "{:?}", result.reasons);
        assert!(result.reasons.iter().any(|r| r.contains("disagree")));
    }

    #[test]
    fn a_required_metric_that_was_not_measured_needs_a_person() {
        let metrics = quiet();
        let result = judge(Judge {
            metrics: &metrics,
            config: &Config::builtin(),
            baseline: None,
            reference_spp: 16,
            reference_noise: 0.0,
            require: &[MetricId::Converge1],
        });
        assert_eq!(result.verdict, Verdict::NeedsHuman);
        assert!(result.reasons.iter().any(|r| r.contains("was required")));
    }

    fn with_scores(raw: f64, blur: f64, ratio: f64) -> FrameMetrics {
        let mut metrics = quiet();
        metrics.mean_rel = Some(raw);
        metrics.p99_rel = Some(raw * 2.5);
        metrics.mean_rel_dark = Some(raw);
        metrics.flip_mean = Some(0.40);
        metrics.flip_p99 = Some(0.70);
        metrics.blur_rel = Some(blur);
        metrics.luma_ratio = Some(ratio);
        metrics.luma_delta = Some((ratio - 1.0).abs());
        metrics
    }

    #[test]
    fn reference_noise_does_not_fail_a_picture_that_matches_after_the_blur() {
        // Owner trial, room-a: ratio 1.03, 9 px blur 0.04, raw 0.35, noise 0.31.
        let metrics = with_scores(0.35, 0.04, 1.03);
        let result = judged(&metrics, None, 64, 0.31);
        assert_eq!(result.verdict, Verdict::Pass, "{:?}", result.reasons);
    }

    #[test]
    fn a_luminance_bias_stays_a_fail_when_the_raw_error_is_only_noise() {
        for (name, ratio) in [("hall", 0.66f64), ("hall-moving", 0.80), ("corner", 1.18)] {
            let delta = (ratio - 1.0).abs();
            let metrics = with_scores(0.35, delta, ratio);
            let result = judged(&metrics, None, 64, 0.31);
            assert_eq!(
                result.verdict,
                Verdict::Fail,
                "{name}: {:?}",
                result.reasons
            );
            assert!(
                result
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("luminance ratio")),
                "{name}: {:?}",
                result.reasons
            );
        }
    }
}
