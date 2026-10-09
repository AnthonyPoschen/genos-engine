//! Debug switches for the picture: what the shader shows, the live lighting knobs,
//! and per-brick and per-lamp reports. The tools crate drives these; a game frame
//! leaves them at their defaults, which change nothing.

use super::Renderer;
use crate::pack::Pack;
use crate::probe_tier::{BrickReport, TierLayout, TierState, TierWeights};

/// What the shaded faces show. `Full` is the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Full,
    /// Lamps and the sun only, shadowed.
    Direct,
    /// The probes' light only (every bounce the bounce limit keeps).
    Bounce,
    /// The near-field rays' answer only; black where no ray met a surface.
    Near,
    /// The probes without the near-field correction.
    Far,
    Albedo,
    Normal,
    /// Distance from the eye, white near, black at 50 m.
    Depth,
    /// Direct light of one lamp ([`DebugView::lamp`]), shadowed.
    Light,
}

impl ViewMode {
    pub const ALL: [ViewMode; 9] = [
        Self::Full,
        Self::Direct,
        Self::Bounce,
        Self::Near,
        Self::Far,
        Self::Albedo,
        Self::Normal,
        Self::Depth,
        Self::Light,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Direct => "direct",
            Self::Bounce => "bounce",
            Self::Near => "near",
            Self::Far => "far",
            Self::Albedo => "albedo",
            Self::Normal => "normal",
            Self::Depth => "depth",
            Self::Light => "light",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == text)
    }

    fn code(self) -> u32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }
}

/// Bounces of probe light the picture shows. The probes keep three cubes: light
/// after one bounce, after up to two, and after every bounce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bounces {
    #[default]
    All,
    /// No probe light: direct only.
    Zero,
    One,
    Two,
}

impl Bounces {
    /// `None` is every bounce.
    pub fn from_limit(limit: Option<u32>) -> Self {
        match limit {
            None => Self::All,
            Some(0) => Self::Zero,
            Some(1) => Self::One,
            Some(2) => Self::Two,
            Some(_) => Self::All,
        }
    }

    pub fn limit(self) -> Option<u32> {
        match self {
            Self::All => None,
            Self::Zero => Some(0),
            Self::One => Some(1),
            Self::Two => Some(2),
        }
    }

    fn code(self) -> u32 {
        match self {
            Self::All => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Zero => 3,
        }
    }
}

/// The shader's debug word (scene block `grid_at.w`): mode in bits 0-7, the lamp in
/// bits 8-23, the bounce cube in bits 24-25.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugView {
    pub mode: ViewMode,
    pub lamp: u32,
    pub bounces: Bounces,
}

impl DebugView {
    pub(crate) fn word(self) -> u32 {
        self.mode.code() | (self.lamp.min(0xFFFF) << 8) | (self.bounces.code() << 24)
    }
}

/// Every lighting knob that can change while the picture runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingConfig {
    /// How the tier ranks its work.
    pub weights: TierWeights,
    /// GPU milliseconds a frame may spend on tier work.
    pub tier_ms: f64,
    /// Near-field rays per pixel; `None` is the shader's own count.
    pub near_rays: Option<u32>,
    /// Time constant of the shown tier light, seconds (0 shows builds as they land).
    pub view_seconds: f32,
    /// Bounces of probe light in the picture.
    pub bounces: Bounces,
    /// Relative change a probe face must pass before an update shows.
    pub notice_band: f32,
    /// Metres between tier probes; `None` fits it to the surfaces in view.
    /// Changing it rebuilds the tier.
    pub spacing: Option<f32>,
}

/// Picture-side debug state the renderer keeps.
#[derive(Default)]
pub(crate) struct DebugState {
    pub view: DebugView,
    pub near_rays: Option<u32>,
    pub spacing: Option<f32>,
}

impl DebugState {
    /// Write the debug word and the overrides into the scene block.
    pub fn apply(&self, pack: &mut Pack) {
        pack.debug_view = self.view.word();
        if let Some(rays) = self.near_rays {
            pack.near_rays = rays.min(64) + 1;
        }
    }
}

/// What one lamp gives the picture, from the CPU side.
/// One live probe's stored light, for the debug tools.
#[derive(Clone, Copy, Debug)]
pub struct ProbeValue {
    pub position: [f32; 3],
    pub samples: f32,
    /// Luminance of each face (+x, -x, +y, -y, +z, -z) of the top cube (every bounce).
    pub top: [f32; 6],
    /// The same for the first cube (one bounce).
    pub first: [f32; 6],
}

#[derive(Clone, Debug)]
pub struct LampReport {
    pub index: usize,
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub directional: bool,
    /// Metres past which it adds nothing (0 for a sun).
    pub range: f32,
    /// Metres from the eye.
    pub distance: f32,
    /// Its reach sphere meets the view frustum.
    pub in_view: bool,
    /// Share of the screen its reach sphere covers, 0 to 1.
    pub screen_share: f32,
    /// Luminance of its colour times its screen share: a rank, not a measurement.
    pub impact: f32,
}

impl Renderer {
    /// What the shaded faces show; the default is the picture.
    pub fn set_debug_view(&mut self, view: DebugView) {
        self.debug.view = view;
    }

    pub fn debug_view(&self) -> DebugView {
        self.debug.view
    }

    pub fn lighting_config(&self) -> LightingConfig {
        LightingConfig {
            weights: self.tier.weights,
            tier_ms: self.tier_ms,
            near_rays: self.debug.near_rays,
            view_seconds: self.view_seconds,
            bounces: self.debug.view.bounces,
            notice_band: crate::probe_tier::notice_band(),
            spacing: self.debug.spacing,
        }
    }

    /// Apply every knob. A new spacing starts the tier again.
    pub fn set_lighting_config(&mut self, config: LightingConfig) {
        self.tier.weights = config.weights;
        self.tier_ms = config.tier_ms.max(0.0);
        self.debug.near_rays = config.near_rays;
        self.view_seconds = config.view_seconds.max(0.0);
        self.debug.view.bounces = config.bounces;
        crate::probe_tier::set_notice_band(config.notice_band);
        let spacing = config.spacing.filter(|s| *s > 0.0);
        if spacing != self.debug.spacing {
            let radius = self.tier.layout.radius;
            let weights = self.tier.weights;
            let fresh = spacing.unwrap_or(self.tier.layout.spacing);
            self.tier = TierState::new(TierLayout::new(fresh, radius));
            self.tier.weights = weights;
            self.debug.spacing = spacing;
        }
    }

    /// Bricks the tier holds, with their state and ranking terms.
    pub fn probe_report(&self) -> Vec<BrickReport> {
        self.tier.brick_reports()
    }

    /// The light each live probe of `brick` holds in the field on screen: the luminance
    /// of each face of its top cube (every bounce) and of its first cube (one bounce),
    /// and its sample count.
    pub fn probe_values(&self, brick: [i32; 3]) -> Vec<ProbeValue> {
        let Some((slot, mask, positions)) = self.tier.slot_of(brick) else {
            return Vec::new();
        };
        let Some(texels) = self.gpu.read_tier_slot(slot) else {
            return Vec::new();
        };
        let luma = |t: [f32; 4]| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
        let stride = crate::probe_tier::PROBE_TEXELS as usize;
        (0..64usize)
            .filter(|p| mask & (1u64 << p) != 0)
            .map(|p| {
                let t = &texels[p * stride..(p + 1) * stride];
                ProbeValue {
                    position: positions[p],
                    samples: t[18][3],
                    top: [0, 1, 2, 3, 4, 5].map(|f| luma(t[f])),
                    first: [0, 1, 2, 3, 4, 5].map(|f| luma(t[12 + f])),
                }
            })
            .collect()
    }

    /// The lamps of `scene` as the picture sees them from `camera`.
    pub fn lamp_report(
        &self,
        scene: &genos_scene::Scene,
        camera: &genos_scene::Camera,
    ) -> Vec<LampReport> {
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let matrix = genos_scene::view_proj(camera, aspect);
        let eye = [camera.position.x, camera.position.y, camera.position.z];
        scene
            .lights
            .iter()
            .enumerate()
            .map(|(index, light)| {
                let d = light.direction;
                let directional = d.x * d.x + d.y * d.y + d.z * d.z > 1.0e-8;
                let position = [light.position.x, light.position.y, light.position.z];
                let range = if directional {
                    0.0
                } else {
                    crate::pack::lamp_range(light.color)
                };
                let distance = if directional {
                    0.0
                } else {
                    (0..3)
                        .map(|i| (position[i] - eye[i]).powi(2))
                        .sum::<f32>()
                        .sqrt()
                };
                let screen_share = if directional {
                    1.0
                } else {
                    sphere_screen_share(&matrix, position, range)
                };
                let luma =
                    0.2126 * light.color[0] + 0.7152 * light.color[1] + 0.0722 * light.color[2];
                LampReport {
                    index,
                    position,
                    color: light.color,
                    directional,
                    range,
                    distance,
                    in_view: screen_share > 0.0,
                    screen_share,
                    impact: luma * screen_share,
                }
            })
            .collect()
    }
}

/// Share of the screen a sphere covers: the clipped box of its projected bound.
fn sphere_screen_share(matrix: &[f32; 16], center: [f32; 3], radius: f32) -> f32 {
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    let mut behind = 0;
    for corner in 0..8 {
        let p = [0, 1, 2].map(|i| {
            center[i]
                + if corner & (1 << i) != 0 {
                    radius
                } else {
                    -radius
                }
        });
        let clip = [0, 1, 3].map(|row| {
            matrix[row] * p[0] + matrix[4 + row] * p[1] + matrix[8 + row] * p[2] + matrix[12 + row]
        });
        if clip[2] <= 1.0e-4 {
            behind += 1;
            continue;
        }
        for k in 0..2 {
            let ndc = clip[k] / clip[2];
            lo[k] = lo[k].min(ndc);
            hi[k] = hi[k].max(ndc);
        }
    }
    if behind == 8 {
        return 0.0;
    }
    if behind > 0 {
        // The bound reaches behind the eye: count the whole screen.
        return 1.0;
    }
    let w = (hi[0].min(1.0) - lo[0].max(-1.0)).max(0.0);
    let h = (hi[1].min(1.0) - lo[1].max(-1.0)).max(0.0);
    (w * h / 4.0).clamp(0.0, 1.0)
}
