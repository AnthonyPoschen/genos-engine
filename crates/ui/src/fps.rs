//! Frame-rate overlay.
//!
//! Off does nothing: no accumulation and no glyphs. On, each frame adds its
//! duration to the open window. The readout is the average of that window, and
//! the glyphs are built only when the window closes (the default is 4 Hz, so
//! about every 250 ms). Frames in between reuse those rectangles.

use crate::panel::Paint;
use crate::text;

const BG: [f32; 3] = [0.04, 0.05, 0.06];
const TEXT: [f32; 3] = [0.93, 0.96, 0.86];
const MARGIN: f32 = 12.0;
const PAD: f32 = 6.0;
/// A window that lands a hair under its period still closes. 15 frames at
/// 1/60 s is 0.25 s in real arithmetic and a shade under that in f32.
const WINDOW_SLACK: f32 = 1.0e-4;

/// Cached frame-rate readout. Construct one and call [`FpsOverlay::note`] once a frame.
#[derive(Clone, Debug)]
pub struct FpsOverlay {
    paints: Vec<Paint>,
    frames: u32,
    seconds: f32,
    published: bool,
    live: bool,
    rebuilds: u32,
    fps: f32,
    frame_ms: f32,
    viewport: [f32; 2],
}

impl Default for FpsOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl FpsOverlay {
    pub fn new() -> Self {
        Self {
            paints: Vec::new(),
            frames: 0,
            seconds: 0.0,
            published: false,
            live: false,
            rebuilds: 0,
            fps: 0.0,
            frame_ms: 0.0,
            viewport: [0.0, 0.0],
        }
    }

    /// Record one frame.
    ///
    /// `enabled` false clears a previous readout on the first such call and
    /// returns. Later disabled calls return before touching the window.
    /// `rate_hz` is clamped to 0.25..=30. `frame_seconds` is the wall time of
    /// this frame. A non-positive duration is ignored.
    pub fn note(&mut self, enabled: bool, rate_hz: f32, frame_seconds: f32, viewport: [f32; 2]) {
        if !enabled {
            if self.live {
                self.clear();
            }
            return;
        }
        if !(frame_seconds > 0.0) {
            return;
        }
        self.live = true;
        self.frames += 1;
        self.seconds += frame_seconds;
        let hz = if rate_hz.is_finite() {
            rate_hz.clamp(0.25, 30.0)
        } else {
            4.0
        };
        let period = 1.0 / hz;
        let due = self.seconds + WINDOW_SLACK >= period;
        let resized = self.viewport != viewport;
        if !due && self.published && !resized {
            return;
        }
        if !due && !self.published {
            return;
        }
        if resized && !due && self.published {
            self.viewport = viewport;
            self.rebuild();
            return;
        }
        if self.frames == 0 || self.seconds <= 0.0 {
            return;
        }
        self.fps = self.frames as f32 / self.seconds;
        self.frame_ms = self.seconds * 1000.0 / self.frames as f32;
        self.frames = 0;
        self.seconds = 0.0;
        self.viewport = viewport;
        self.published = true;
        self.rebuild();
    }

    /// Glyphs and the plate behind them. Empty when the overlay is off or the
    /// first window has not closed.
    pub fn paints(&self) -> &[Paint] {
        &self.paints
    }

    /// Last published frames per second and frame milliseconds. `None` until
    /// the first window closes, and `None` again after the overlay is turned off.
    pub fn readout(&self) -> Option<(f32, f32)> {
        if self.published {
            Some((self.fps, self.frame_ms))
        } else {
            None
        }
    }

    /// Times the readout string was turned into rectangles.
    pub fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    fn clear(&mut self) {
        self.paints.clear();
        self.frames = 0;
        self.seconds = 0.0;
        self.published = false;
        self.live = false;
        self.fps = 0.0;
        self.frame_ms = 0.0;
    }

    fn rebuild(&mut self) {
        let label = format!("{:.0} FPS  {:.1} MS", self.fps, self.frame_ms);
        let text_w = label.chars().count() as f32 * text::CELL_W;
        let width = text_w + PAD * 2.0;
        let height = text::CELL_H + PAD * 2.0;
        let x = (self.viewport[0] - width - MARGIN).max(0.0);
        let y = (self.viewport[1] - height - MARGIN).max(0.0);
        let mut paints = Vec::with_capacity(64);
        paints.push(Paint {
            x,
            y,
            w: width,
            h: height,
            color: BG,
        });
        for blot in text::blots(&label, x + PAD, y + PAD, text_w) {
            paints.push(Paint {
                x: blot.x,
                y: blot.y,
                w: blot.w,
                h: blot.h,
                color: TEXT,
            });
        }
        self.paints = paints;
        self.rebuilds += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const VIEW: [f32; 2] = [320.0, 180.0];
    const FRAME: f32 = 1.0 / 60.0;

    #[test]
    fn disabled_skips_and_a_window_rebuilds_once() {
        let mut overlay = FpsOverlay::new();
        for _ in 0..14 {
            overlay.note(true, 4.0, FRAME, VIEW);
        }
        assert!(
            overlay.paints().is_empty(),
            "the first window is still open"
        );
        assert_eq!(overlay.rebuilds(), 0);
        assert!(overlay.readout().is_none());

        overlay.note(true, 4.0, FRAME, VIEW);
        assert_eq!(overlay.rebuilds(), 1);
        let plate_y = overlay.paints()[0].y;
        assert!(
            plate_y > VIEW[1] * 0.5,
            "the plate sits along the bottom, y {plate_y}"
        );
        let (fps, ms) = overlay.readout().unwrap();
        assert!((fps - 60.0).abs() < 0.2, "fps {fps}");
        assert!((ms - 16.7).abs() < 0.2, "ms {ms}");
        assert!(overlay.paints().len() > 8);
        let held = overlay.paints().to_vec();

        for _ in 0..14 {
            overlay.note(true, 4.0, FRAME, VIEW);
        }
        assert_eq!(
            overlay.rebuilds(),
            1,
            "glyphs stay cached inside the window"
        );
        assert_eq!(overlay.paints().len(), held.len());
        for (paint, kept) in overlay.paints().iter().zip(&held) {
            assert_eq!(
                (paint.x, paint.y, paint.w, paint.h, paint.color),
                (kept.x, kept.y, kept.w, kept.h, kept.color)
            );
        }

        overlay.note(true, 4.0, FRAME, VIEW);
        assert_eq!(overlay.rebuilds(), 2);

        overlay.note(false, 4.0, FRAME, VIEW);
        assert!(overlay.paints().is_empty());
        assert!(overlay.readout().is_none());
        let rebuilds = overlay.rebuilds();
        for _ in 0..1_000 {
            overlay.note(false, 4.0, FRAME, VIEW);
        }
        assert_eq!(overlay.rebuilds(), rebuilds);
        assert!(overlay.paints().is_empty());
    }

    #[test]
    fn disabled_notes_cost_less_than_a_live_counter() {
        let mut off = FpsOverlay::new();
        let start = Instant::now();
        for _ in 0..100_000 {
            off.note(false, 4.0, FRAME, VIEW);
        }
        let off_ns = start.elapsed().as_nanos();
        assert_eq!(off.rebuilds(), 0);
        assert!(off.paints().is_empty());

        let mut on = FpsOverlay::new();
        let start = Instant::now();
        for _ in 0..100_000 {
            on.note(true, 4.0, FRAME, VIEW);
        }
        let on_ns = start.elapsed().as_nanos();
        // 100_000 / 60 s at 4 Hz is a bit over 6_000 windows.
        let rebuilds = on.rebuilds();
        assert!(
            (6_000..7_200).contains(&rebuilds),
            "rebuilds {rebuilds} (want about 4 Hz, not once a frame)"
        );
        assert!(
            off_ns * 4 < on_ns.max(1) || off_ns < 20_000_000,
            "disabled path took {off_ns} ns against a live counter at {on_ns} ns"
        );
        eprintln!(
            "fps overlay cost: off {off_ns} ns, on {on_ns} ns, rebuilds {rebuilds} for 100000 frames"
        );
    }
}
