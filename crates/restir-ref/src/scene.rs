//! Built-in receiver grids. Distances are metres. Y is up.

use genos_math::Vec3;

use crate::reservoir::m_as_f32;
use crate::shade::{Aabb, Lamp, Receiver, Scene};

fn as_f32(value: usize) -> f32 {
    m_as_f32(u32::try_from(value).unwrap_or(u32::MAX))
}

fn grid(width: usize, height: usize, spacing: f32, albedo: Vec3) -> Vec<Receiver> {
    let x0 = -spacing * (as_f32(width) - 1.0) * 0.5;
    let z0 = -spacing * (as_f32(height) - 1.0) * 0.5;
    let mut receivers = Vec::with_capacity(width.saturating_mul(height));
    for row in 0..height {
        for col in 0..width {
            receivers.push(Receiver {
                position: Vec3::new(x0 + spacing * as_f32(col), 0.0, z0 + spacing * as_f32(row)),
                normal: Vec3::Y,
                albedo,
            });
        }
    }
    receivers
}

impl Scene {
    /// One receiver, eight coloured lamps, no occluder.
    /// A target-proportional proposal is exact in luminance on this scene.
    #[must_use]
    pub fn open_row() -> Self {
        let mut lamps = Vec::new();
        for index in 0..8 {
            let x = -1.4 + 0.4 * as_f32(index);
            let tint = if index % 2 == 0 {
                Vec3::new(1.0, 0.45, 0.2)
            } else {
                Vec3::new(0.25, 0.55, 1.0)
            };
            lamps.push(Lamp {
                position: Vec3::new(x, 1.2 + 0.15 * as_f32(index % 3), 0.4),
                intensity: tint * (2.0 + as_f32(index)),
            });
        }
        Self {
            lamps,
            receivers: vec![Receiver {
                position: Vec3::ZERO,
                normal: Vec3::Y,
                albedo: Vec3::new(0.8, 0.7, 0.6),
            }],
            occluders: Vec::new(),
            width: 1,
            camera: Vec3::new(0.0, 1.6, -2.0),
        }
    }

    /// 4×4 open grid, sixteen lamps. Uniform proposal, no shadows.
    #[must_use]
    pub fn open_grid() -> Self {
        let mut lamps = Vec::new();
        for row in 0..4 {
            for col in 0..4 {
                lamps.push(Lamp {
                    position: Vec3::new(
                        -1.5 + as_f32(col),
                        1.5 + 0.25 * as_f32(row),
                        -1.5 + as_f32(row),
                    ),
                    intensity: Vec3::new(1.0, 0.9, 0.8) * (1.0 + as_f32(col + row)),
                });
            }
        }
        Self {
            lamps,
            receivers: grid(4, 4, 0.5, Vec3::new(0.75, 0.75, 0.75)),
            occluders: Vec::new(),
            width: 4,
            camera: Vec3::new(0.0, 1.6, -2.5),
        }
    }

    /// One receiver and twenty lamps whose intensities span two decades.
    #[must_use]
    pub fn variance_point() -> Self {
        let mut lamps = Vec::new();
        for index in 0..20 {
            let angle = as_f32(index) * 0.31;
            lamps.push(Lamp {
                position: Vec3::new(
                    angle.sin() * 1.5,
                    0.8 + 0.05 * as_f32(index),
                    angle.cos() * 1.5,
                ),
                intensity: Vec3::new(0.15, 0.15, 0.15) * as_f32(index + 1),
            });
        }
        Self {
            lamps,
            receivers: vec![Receiver {
                position: Vec3::ZERO,
                normal: Vec3::Y,
                albedo: Vec3::new(0.8, 0.8, 0.8),
            }],
            occluders: Vec::new(),
            width: 1,
            camera: Vec3::new(0.0, 1.6, -2.0),
        }
    }

    /// Two receivers and a near lamp, so neighbour targets disagree.
    #[must_use]
    pub fn domain_pair() -> Self {
        Self {
            lamps: vec![
                Lamp {
                    position: Vec3::new(0.0, 0.35, 0.0),
                    intensity: Vec3::new(80.0, 80.0, 80.0),
                },
                Lamp {
                    position: Vec3::new(0.8, 3.5, 0.0),
                    intensity: Vec3::new(6.0, 6.0, 6.0),
                },
            ],
            receivers: vec![
                Receiver {
                    position: Vec3::ZERO,
                    normal: Vec3::Y,
                    albedo: Vec3::new(1.0, 1.0, 1.0),
                },
                Receiver {
                    position: Vec3::new(1.6, 0.0, 0.0),
                    normal: Vec3::Y,
                    albedo: Vec3::new(1.0, 1.0, 1.0),
                },
            ],
            occluders: Vec::new(),
            width: 2,
            camera: Vec3::new(0.8, 1.5, -2.0),
        }
    }

    /// 8×8 grid, twelve lamps, no occluder. The harness gates this scene.
    ///
    /// A target-proportional proposal makes each receiver's luminance the
    /// integral on every sample, so the multi-seed mean holds still.
    #[must_use]
    pub fn gate() -> Self {
        let mut lamps = Vec::new();
        for row in 0..3 {
            for col in 0..4 {
                let low = col == 0;
                let height = if low { 0.55 } else { 2.8 };
                let tint = if (col + row) % 2 == 0 {
                    Vec3::new(1.0, 0.85, 0.7)
                } else {
                    Vec3::new(0.7, 0.85, 1.0)
                };
                lamps.push(Lamp {
                    position: Vec3::new(-1.8 + 1.2 * as_f32(col), height, -1.2 + 1.2 * as_f32(row)),
                    intensity: tint * (3.0 + as_f32(col + row)),
                });
            }
        }
        Self {
            lamps,
            receivers: grid(8, 8, 0.45, Vec3::new(0.72, 0.72, 0.72)),
            occluders: Vec::new(),
            width: 8,
            camera: Vec3::new(0.0, 1.7, -3.0),
        }
    }

    /// [`Self::gate`] plus a wall that cuts the low lamps on the first column.
    ///
    /// Visibility is one sample per chain. The mean over seeds is the bias
    /// check. That mean stays spatially white, so it is not the checkerboard gate.
    #[must_use]
    pub fn shadow() -> Self {
        let mut scene = Self::gate();
        scene.occluders.push(Aabb {
            min: Vec3::new(-0.15, 0.0, -2.0),
            max: Vec3::new(0.15, 1.3, 2.0),
        });
        scene
    }
}
