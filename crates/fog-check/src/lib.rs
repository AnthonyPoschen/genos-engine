//! Metric harness for fog and volumetric lighting.
//!
//! Closed-form fixtures are graded here. Renderer metrics stay
//! `NOT-IMPLEMENTED` until the froxel pass and the media path tracer exist.
//! This crate does not draw and does not allocate a froxel.

mod analytic;
mod cli;
mod feature;
mod report;

pub use cli::{resolve, Config, HELP};
pub use report::{exit_code, format_lines, judge, write_metrics, Metric, Report, Status};

/// Which metrics to score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    /// Closed forms in `genos_math`, plus a synthetic picture.
    Analytic,
    /// Renderer metrics. Each one is `NOT-IMPLEMENTED` until that pass exists.
    Feature,
    /// Analytic metrics, then feature metrics.
    All,
}

/// Analytic input. Feature metrics ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    /// A candidate that matches the closed form.
    Good,
    /// A candidate that breaks the closed form on purpose.
    Broken,
}

impl Suite {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Analytic => "analytic",
            Self::Feature => "feature",
            Self::All => "all",
        }
    }
}

impl Fixture {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Broken => "broken",
        }
    }
}

/// Score one suite. Does not touch the disk.
#[must_use]
pub fn evaluate(suite: Suite, fixture: Fixture) -> Report {
    let mut metrics = Vec::new();
    if matches!(suite, Suite::Analytic | Suite::All) {
        metrics.extend(analytic::evaluate_analytic(fixture));
    }
    if matches!(suite, Suite::Feature | Suite::All) {
        metrics.extend(feature::evaluate_feature());
    }
    Report {
        suite,
        fixture,
        metrics,
    }
}
