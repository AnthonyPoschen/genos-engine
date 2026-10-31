//! Preset grid bytes and the sample budget a later auto-tune can write.
//!
//! VRAM is `froxels * 16 + columns * 8`, decimal MB. The 16 bytes reserve the
//! history slot. History is not blended. The frame dispatches
//! [`dispatched_grid`]: zero axes keep the preset, and a positive `samples`
//! limits cells. [`fit_pass`] is the CPU helper for that limit. The frame
//! does not call it. Potato does not dispatch.

use genos_scene::{FogSettings, LightingPreset};

use crate::fog::{dispatched_grid, grid_size, GridSize};

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
pub fn unfitted_milliseconds(
    preset: LightingPreset,
    mut time_groups: impl FnMut(u32) -> f64,
) -> f64 {
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

/// Bytes of the preset grid. Zero when the preset allocates no froxel.
#[must_use]
pub fn allocation_bytes(preset: LightingPreset) -> u64 {
    byte_count(grid_size(preset))
}

/// Counters the debug page and `get_fog_stats` share.
///
/// [`fog_probe`] leaves `pass_ms` at 0. `Renderer::fog_frame_probe` writes the
/// last frame's lavapipe timestamp when the froxel dispatched. That number is
/// not an RTX 4070 measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogProbe {
    /// 1 when the pass would run, otherwise 0.
    pub dispatches: u32,
    /// The `samples` setting. Zero dispatches every cell of the resolved grid.
    pub samples_used: u32,
    /// Cells dropped before the dispatch. The frame dispatches `dispatched_*`,
    /// so this stays 0.
    pub samples_capped: u32,
    /// `grid_width` setting. Zero keeps the preset width.
    pub grid_width: u32,
    /// `grid_height` setting. Zero keeps the preset height.
    pub grid_height: u32,
    /// `grid_slices` setting. Zero keeps the preset slice count.
    pub grid_slices: u32,
    /// Columns the frame dispatches.
    pub dispatched_width: u32,
    /// Rows the frame dispatches.
    pub dispatched_height: u32,
    /// Slices the frame dispatches.
    pub dispatched_slices: u32,
    /// History accepts. History is off, so this stays 0.
    pub history_accepts: u32,
    /// History rejects. History is off, so this stays 0.
    pub history_rejects: u32,
    /// Grid plus column bytes when the pass runs, otherwise 0.
    pub vram_bytes: u64,
    /// Last frame's froxel timestamp, milliseconds. [`fog_probe`] leaves this 0.
    pub pass_ms: f64,
}

/// Probe for `fog` on `preset`. A density of zero with no volumes reports
/// `dispatches` 0 and zero bytes, including when `history` is on.
#[must_use]
pub fn fog_probe(fog: &FogSettings, preset: LightingPreset) -> FogProbe {
    let size = dispatched_grid(fog, preset);
    let live = fog.dispatches();
    FogProbe {
        dispatches: u32::from(live),
        samples_used: fog.samples,
        samples_capped: 0,
        history_accepts: 0,
        history_rejects: 0,
        vram_bytes: if live { byte_count(size) } else { 0 },
        pass_ms: 0.0,
        grid_width: fog.grid_width,
        grid_height: fog.grid_height,
        grid_slices: fog.grid_slices,
        dispatched_width: size.width,
        dispatched_height: size.height,
        dispatched_slices: size.slices,
    }
}

fn byte_count(size: GridSize) -> u64 {
    let width = u64::from(size.width);
    let height = u64::from(size.height);
    let slices = u64::from(size.slices);
    let froxels = width.saturating_mul(height).saturating_mul(slices);
    let columns = width.saturating_mul(height);
    froxels
        .saturating_mul(16)
        .saturating_add(columns.saturating_mul(8))
}

fn megabytes(size: GridSize) -> f64 {
    let bytes = u32::try_from(byte_count(size)).unwrap_or(u32::MAX);
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

    #[test]
    fn a_zero_density_and_a_history_flag_do_not_dispatch_or_accept() {
        let quiet = FogSettings {
            enabled: true,
            ..FogSettings::default()
        };
        let probe = fog_probe(&quiet, LightingPreset::High);
        assert_eq!(probe.dispatches, 0);
        assert_eq!(probe.vram_bytes, 0);
        assert_eq!(probe.samples_used, 0);
        let live = FogSettings {
            enabled: true,
            density: 0.08,
            history: true,
            ..FogSettings::default()
        };
        let probe = fog_probe(&live, LightingPreset::Low);
        assert_eq!(probe.dispatches, 1);
        assert_eq!(probe.history_accepts, 0);
        assert_eq!(probe.history_rejects, 0);
        assert_eq!(probe.pass_ms, 0.0);
        assert_eq!(probe.samples_capped, 0);
        assert_eq!(probe.samples_used, 0);
        assert_eq!(probe.grid_width, 0);
        assert_eq!(probe.dispatched_width, 64);
        assert_eq!(probe.dispatched_height, 36);
        assert_eq!(probe.dispatched_slices, 24);
        assert_eq!(probe.vram_bytes, 903_168);
        let tuned = FogSettings {
            enabled: true,
            density: 0.08,
            grid_width: 8,
            grid_height: 4,
            grid_slices: 2,
            samples: 10,
            ..FogSettings::default()
        };
        let tuned_probe = fog_probe(&tuned, LightingPreset::High);
        assert_eq!(tuned_probe.dispatches, 1);
        assert_eq!(tuned_probe.samples_used, 10);
        assert_eq!(
            (
                tuned_probe.dispatched_width,
                tuned_probe.dispatched_height,
                tuned_probe.dispatched_slices
            ),
            (8, 1, 1)
        );
        assert!(tuned_probe.vram_bytes < 903_168);
    }
}
