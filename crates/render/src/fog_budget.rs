//! Preset grid bytes and the sample cap that keeps a pass under its ceiling.
//!
//! VRAM is `froxels * 16 + columns * 8`, decimal MB. The 16 bytes reserve the
//! history slot. History is not blended. A pass over its millisecond ceiling
//! loses samples first, then far slices. Near slices stay. Potato does not
//! dispatch.

use genos_scene::LightingPreset;

use crate::fog::{grid_size, GridSize};

/// One preset after the controller has run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassCost {
    /// Allocated grid plus column target, decimal MB.
    pub vram_mb: f64,
    /// In-scatter samples kept this frame.
    pub samples: u32,
    /// Slices kept. Far slices drop only after the sample cap is 1.
    pub slices: u32,
    /// Measured or predicted pass time, milliseconds.
    pub milliseconds: f64,
}

/// Decimal MB of the preset grid. Potato, day-cycle, and one-bounce allocate 0.
#[must_use]
pub fn allocation_mb(preset: LightingPreset) -> f64 {
    megabytes(grid_size(preset))
}

/// The High grid counted twice. That misses the 12 MB ceiling.
#[must_use]
pub fn allocation_mb_doubled() -> f64 {
    allocation_mb(LightingPreset::High) * 2.0
}

/// In-scatter sample cap before the controller shrinks it.
#[must_use]
pub fn sample_cap(preset: LightingPreset) -> u32 {
    match preset {
        LightingPreset::Potato | LightingPreset::DayCycle | LightingPreset::Gi1Bounce => 0,
        LightingPreset::Low => 4_000,
        LightingPreset::Medium => 12_000,
        LightingPreset::High => 32_000,
    }
}

/// Millisecond ceiling. These are the 4070 figures. A lavapipe run is labelled
/// as lavapipe and is not one of these cards.
#[must_use]
pub fn millisecond_ceiling(preset: LightingPreset) -> f64 {
    match preset {
        LightingPreset::Potato | LightingPreset::DayCycle | LightingPreset::Gi1Bounce => 0.05,
        LightingPreset::Low => 0.40,
        LightingPreset::Medium => 0.80,
        LightingPreset::High => 1.20,
    }
}

/// Full sample cap, timed and not reduced. The ceiling test uses this as the
/// broken pass.
#[must_use]
pub fn unfitted_milliseconds(preset: LightingPreset, mut time_groups: impl FnMut(u32) -> f64) -> f64 {
    let samples = sample_cap(preset);
    if samples == 0 {
        0.0
    } else {
        time_groups(samples)
    }
}

/// Shrink samples, then far slices, until `time_groups` is inside the ceiling
/// or only the near slice at one sample remains.
#[must_use]
pub fn fit_pass(preset: LightingPreset, mut time_groups: impl FnMut(u32) -> f64) -> PassCost {
    let size = grid_size(preset);
    let ceiling = millisecond_ceiling(preset);
    let mut samples = sample_cap(preset);
    let mut slices = size.slices;
    if samples == 0 || slices == 0 {
        return PassCost {
            vram_mb: megabytes(size),
            samples: 0,
            slices: 0,
            milliseconds: 0.0,
        };
    }
    loop {
        let milliseconds = time_groups(samples);
        if milliseconds <= ceiling || (samples <= 1 && slices <= 1) {
            return PassCost {
                vram_mb: megabytes(GridSize {
                    width: size.width,
                    height: size.height,
                    slices,
                }),
                samples,
                slices,
                milliseconds,
            };
        }
        if samples > 1 {
            samples /= 2;
        } else {
            slices = slices.saturating_sub(1).max(1);
        }
    }
}

fn megabytes(size: GridSize) -> f64 {
    let width = u64::from(size.width);
    let height = u64::from(size.height);
    let slices = u64::from(size.slices);
    let froxels = width.saturating_mul(height).saturating_mul(slices);
    let columns = width.saturating_mul(height);
    let bytes = froxels.saturating_mul(16).saturating_add(columns.saturating_mul(8));
    let bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
    f64::from(bytes) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_bytes_match_the_design_table() {
        assert_eq!(allocation_mb(LightingPreset::Potato), 0.0);
        assert!((allocation_mb(LightingPreset::Low) - 0.903_168).abs() < 1.0e-9);
        assert!((allocation_mb(LightingPreset::Medium) - 2.695_680).abs() < 1.0e-9);
        assert!((allocation_mb(LightingPreset::High) - 11.174_400).abs() < 1.0e-9);
        assert!(allocation_mb_doubled() > 12.0);
    }

    #[test]
    fn an_over_ceiling_cap_loses_samples_before_slices() {
        let raw = unfitted_milliseconds(LightingPreset::High, |groups| f64::from(groups) * 0.000_1);
        assert!(raw > 1.20, "{raw}");
        let fitted = fit_pass(LightingPreset::High, |groups| f64::from(groups) * 0.000_1);
        assert!(fitted.milliseconds <= 1.20, "{fitted:?}");
        assert!(fitted.samples < 32_000, "{fitted:?}");
        assert_eq!(fitted.slices, 48, "{fitted:?}");
        assert!((fitted.vram_mb - 11.174_400).abs() < 1.0e-9);
    }

    #[test]
    fn potato_does_not_dispatch() {
        let mut dispatched = false;
        let fitted = fit_pass(LightingPreset::Potato, |_| {
            dispatched = true;
            0.0
        });
        assert!(!dispatched);
        assert_eq!(fitted.milliseconds, 0.0);
        assert_eq!(fitted.samples, 0);
        assert_eq!(fitted.vram_mb, 0.0);
    }
}
