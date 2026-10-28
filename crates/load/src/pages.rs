//! Texture pages and the miss rule.
//!
//! A page that is not resident is the finest coarser mip that is, or the factor
//! colour. It is never black and never a half-written page. Which mips stay
//! resident is [`crate::stream`], inside the VRAM budget. `--watch` flips to
//! the new page on the next sample, after the upload has finished.

/// `resident[0]` is the finest mip. A hole is `None`.
#[must_use]
pub fn miss_colour(resident: &[Option<[f32; 4]>], wanted: usize, factor: [f32; 4]) -> [f32; 4] {
    resident
        .iter()
        .skip(wanted)
        .flatten()
        .copied()
        .next()
        .unwrap_or(factor)
}

/// The published page and a page that is still uploading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialWatch {
    published: u64,
    incoming: Option<u64>,
    frames_since_flip: u32,
    flipped: bool,
}

impl MaterialWatch {
    #[must_use]
    pub fn new(published: u64) -> Self {
        Self {
            published,
            incoming: None,
            frames_since_flip: 0,
            flipped: false,
        }
    }

    pub fn begin(&mut self, hash: u64) {
        self.incoming = Some(hash);
        self.flipped = false;
    }

    /// The sample keeps the old page until [`Self::finish_upload`].
    #[must_use]
    pub fn sample(&self) -> u64 {
        self.published
    }

    pub fn finish_upload(&mut self) {
        if let Some(hash) = self.incoming.take() {
            self.published = hash;
            self.frames_since_flip = 0;
            self.flipped = true;
        }
    }

    /// One frame after the flip. The upload frames are not counted.
    pub fn frame(&mut self) {
        if self.flipped && self.incoming.is_none() {
            self.frames_since_flip = self.frames_since_flip.saturating_add(1);
        }
    }

    #[must_use]
    pub fn frames_since_flip(&self) -> u32 {
        self.frames_since_flip
    }
}

/// `--watch` reload. The old page stays published until the upload finishes,
/// then the next frame is the one that sees the new page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotReload {
    watch: MaterialWatch,
    pending: u64,
}

impl HotReload {
    #[must_use]
    pub fn open(published: u64) -> Self {
        Self {
            watch: MaterialWatch::new(published),
            pending: published,
        }
    }

    /// Upload `hash` and publish it. The frame counter starts at the flip.
    pub fn upload(&mut self, hash: u64) {
        self.pending = hash;
        self.watch.begin(hash);
        self.watch.finish_upload();
    }

    /// Start an upload and never publish it. The sample stays on the old page.
    pub fn hold(&mut self, hash: u64) {
        self.pending = hash;
        self.watch.begin(hash);
    }

    /// Frames from the flip until the sample sees [`Self::upload`]'s hash.
    /// Stops at `limit` when the page never appears.
    #[must_use]
    pub fn frames_until_visible(&mut self, limit: u32) -> f32 {
        let mut left = limit;
        while left > 0 {
            self.watch.frame();
            left -= 1;
            if self.watch.sample() == self.pending && self.watch.frames_since_flip() >= 1 {
                return frames_f32(self.watch.frames_since_flip());
            }
        }
        frames_f32(limit)
    }
}

#[expect(clippy::as_conversions, reason = "a reload waits a handful of frames")]
#[expect(clippy::cast_precision_loss, reason = "a reload waits a handful of frames")]
fn frames_f32(value: u32) -> f32 {
    value as f32
}

#[expect(clippy::as_conversions, reason = "a residency total in MiB fits the f32 mantissa")]
#[expect(clippy::cast_precision_loss, reason = "a residency total in MiB fits the f32 mantissa")]
#[must_use]
pub fn mib_f32(bytes: u64) -> f32 {
    let whole = bytes / (1024 * 1024);
    whole as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_miss_uses_the_coarser_mip_or_the_factor() {
        let factor = [0.2, 0.0, 0.0, 1.0];
        let resident = [None, Some([1.0, 0.0, 0.0, 1.0])];
        assert_eq!(miss_colour(&resident, 0, factor), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(miss_colour(&[None, None], 0, factor), factor);
        assert_ne!(miss_colour(&[None, None], 0, factor), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_held_upload_never_reaches_the_new_page() {
        let mut held = HotReload::open(1);
        held.hold(2);
        let stalled = held.frames_until_visible(5);
        assert!((stalled - 5.0).abs() < 1.0e-6, "{stalled}");
        let mut live = HotReload::open(1);
        live.upload(2);
        let flipped = live.frames_until_visible(5);
        assert!((flipped - 1.0).abs() < 1.0e-6, "{flipped}");
    }

    #[test]
    fn the_new_page_is_visible_on_the_next_frame() {
        let mut watch = MaterialWatch::new(1);
        watch.begin(2);
        assert_eq!(watch.sample(), 1);
        watch.frame();
        assert_eq!(watch.frames_since_flip(), 0);
        watch.finish_upload();
        assert_eq!(watch.sample(), 2);
        watch.frame();
        assert_eq!(watch.frames_since_flip(), 1);
    }
}
