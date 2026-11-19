//! Presented-frame read for lighting and render picture gates.
//!
//! A gate grades these pixels. `draw` with readback returns the swapchain
//! image, tightly packed BGRA8.

use genos_render::{Renderer, World};
use genos_scene::Camera;
use genos_window::Window;

const LUMA_B: f64 = 0.0722;
const LUMA_G: f64 = 0.7152;
const LUMA_R: f64 = 0.2126;

pub struct PresentedFrame {
    pub pixels: Vec<u8>,
}

impl PresentedFrame {
    pub fn from_draw(
        window: &mut Window,
        renderer: &mut Renderer,
        world: &World,
        camera: &Camera,
    ) -> Self {
        let _ = window.pump();
        let pixels = renderer
            .draw(world, camera, true)
            .expect("draw")
            .expect("readback: a lighting/render gate must read the presented image");
        assert!(!pixels.is_empty(), "presented image was empty");
        let width = renderer.width();
        let height = renderer.height();
        let expected = usize::try_from(width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(height).unwrap_or(0))
            .saturating_mul(4);
        assert_eq!(
            pixels.len(),
            expected,
            "presented image size does not match the surface"
        );
        let _ = renderer.gpu_kind();
        Self { pixels }
    }

    #[allow(
        dead_code,
        reason = "the compose gates sum these bytes; the grain gate reads luma"
    )]
    pub fn sum_bytes(&self) -> u64 {
        self.pixels.iter().map(|&b| u64::from(b)).sum()
    }

    /// Rec.709 luma of the presented BGRA image, in display codes (0..255).
    #[allow(
        dead_code,
        reason = "the moving-lamp gate reads luma; the compose gates sum bytes"
    )]
    pub fn luma(&self) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.pixels.len() / 4);
        let mut i = 0usize;
        while i + 3 < self.pixels.len() {
            let b = f64::from(self.pixels[i]);
            let g = f64::from(self.pixels[i + 1]);
            let r = f64::from(self.pixels[i + 2]);
            out.push(LUMA_B * b + LUMA_G * g + LUMA_R * r);
            i += 4;
        }
        out
    }
}
