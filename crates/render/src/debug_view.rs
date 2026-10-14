//! Debug switches for the picture: what the lighting's compose pass shows, and
//! per-lamp reports. The tools crate drives these; a game frame leaves them at
//! their defaults, which change nothing.

use super::Renderer;
use crate::pack::Pack;

/// What the picture shows. `Full` is the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Full,
    /// Lamps and the sun only, shadowed.
    Direct,
    /// Bounced light only.
    Bounce,
    Albedo,
    Normal,
    /// Distance from the eye, white near, black at 50 m.
    Depth,
    /// Direct light only, as `Direct`; kept for the tools' lamp view.
    Light,
    /// Surface cache: the texel size each pixel reads (red 6.25 cm, green
    /// 12.5 cm, blue 25 cm, magenta coarser), dimmed where the texel holds fewer
    /// than 4 sets; shaded by the normal so surfaces read.
    Texels,
    /// Short-term relative standard deviation of the cache entry under each
    /// pixel. Grey is a missing or unallocated entry. Blue is quiet, red is noisy.
    Variance,
}

impl ViewMode {
    /// In the compose pass's code order (lighting_compose.comp).
    pub const ALL: [ViewMode; 9] = [
        Self::Full,
        Self::Direct,
        Self::Bounce,
        Self::Albedo,
        Self::Normal,
        Self::Depth,
        Self::Light,
        Self::Texels,
        Self::Variance,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Direct => "direct",
            Self::Bounce => "bounce",
            Self::Albedo => "albedo",
            Self::Normal => "normal",
            Self::Depth => "depth",
            Self::Light => "light",
            Self::Texels => "texels",
            Self::Variance => "variance",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == text)
    }

    fn code(self) -> u32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }
}

/// The compose pass's debug word (scene block `grid_at.w`): mode in bits 0-7, the
/// lamp in bits 8-23. Bits 24 and 25 are the unfilled detector (count, tint),
/// written by [`DebugState::apply`], not by [`DebugView::word`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugView {
    pub mode: ViewMode,
    pub lamp: u32,
}

impl DebugView {
    pub(crate) fn word(self) -> u32 {
        self.mode.code() | (self.lamp.min(0xFFFF) << 8)
    }
}

/// Picture-side debug state the renderer keeps.
#[derive(Default)]
pub(crate) struct DebugState {
    pub view: DebugView,
    pub unfilled: crate::unfilled::UnfilledControl,
}

impl DebugState {
    /// Write the debug word into the scene block.
    pub fn apply(&self, pack: &mut Pack) {
        let mut word = self.view.word();
        if self.unfilled.count {
            word |= 1 << 24;
        }
        if self.unfilled.tint {
            word |= 1 << 25;
        }
        pack.debug_view = word;
    }
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
    /// Count unfilled surface-cache pixels, and optionally tint or dump them.
    /// Tint or a dump threshold turns counting on.
    pub fn set_unfilled(&mut self, mut control: crate::unfilled::UnfilledControl) {
        if control.tint || control.dump_pct.is_some() {
            control.count = true;
        }
        self.debug.unfilled = control;
        self.gpu.unfilled = control;
    }

    pub fn unfilled(&self) -> crate::unfilled::UnfilledControl {
        self.gpu.unfilled
    }

    /// `--out` when this process was started with that argument.
    pub fn debug_out_cli(&self) -> Option<&str> {
        self.gpu.out_cli.as_deref()
    }

    /// Debug output directory in force: the scene value, then `GENOS_UNFILLED_DIR`
    /// when `--out` was not passed, then `--out`.
    pub fn debug_settings(&self, scene: &genos_scene::Scene) -> genos_scene::DebugSettings {
        scene.debug.effective(self.gpu.out_cli.as_deref())
    }

    /// Counters from the last submitted frame. Waits until that frame's copy is visible.
    pub fn unfilled_stats(&mut self) -> crate::unfilled::UnfilledStats {
        unsafe {
            (self.gpu.fns.device_wait)(self.gpu.device);
        }
        let slot = self.gpu.lighting.last_slot;
        self.gpu.take_unfilled(slot);
        self.gpu.last_unfilled.unwrap_or_default()
    }

    /// What the picture shows; the default is the picture.
    pub fn set_debug_view(&mut self, view: DebugView) {
        self.debug.view = view;
    }

    pub fn debug_view(&self) -> DebugView {
        self.debug.view
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
