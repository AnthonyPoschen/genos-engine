//! Texture pages, the residency cap, and the miss rule.
//!
//! A page that is not resident is the finest coarser mip that is, or the factor
//! colour. It is never black and never a half-written page. When the budget is
//! full, the farthest image's finest mip is the one that drops. `--watch` flips
//! to the new page on the next sample, after the upload has finished.

/// One image's mip sizes in bytes, finest first. `rank` is distance: higher drops first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageChain {
    pub levels: Vec<u64>,
    pub rank: u32,
}

/// Bytes that stay under `cap`, dropping finest mips of the farthest images.
#[must_use]
pub fn resident_bytes(images: &[ImageChain], cap: u64) -> u64 {
    let mut dropped = vec![0_usize; images.len()];
    loop {
        let sum = kept_bytes(images, &dropped);
        if sum <= cap {
            return sum;
        }
        let mut choice: Option<usize> = None;
        for (index, image) in images.iter().enumerate() {
            if dropped[index] >= image.levels.len() {
                continue;
            }
            let replace = match choice {
                None => true,
                Some(best) => {
                    image.rank > images[best].rank
                        || (image.rank == images[best].rank
                            && image.levels[dropped[index]] > images[best].levels[dropped[best]])
                }
            };
            if replace {
                choice = Some(index);
            }
        }
        let Some(index) = choice else {
            return sum;
        };
        dropped[index] += 1;
    }
}

fn kept_bytes(images: &[ImageChain], dropped: &[usize]) -> u64 {
    images
        .iter()
        .zip(dropped)
        .map(|(image, skip)| image.levels.iter().skip(*skip).sum::<u64>())
        .sum()
}

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

    fn mib(n: u64) -> u64 {
        n * 1024 * 1024
    }

    #[test]
    fn the_farthest_finest_mip_drops_until_the_cap() {
        let images = [
            ImageChain {
                levels: vec![mib(300), mib(75), mib(20)],
                rank: 2,
            },
            ImageChain {
                levels: vec![mib(10)],
                rank: 0,
            },
        ];
        let resident = resident_bytes(&images, mib(256));
        assert!(resident <= mib(256), "{resident}");
        assert_eq!(resident, mib(105));
        assert!(images.iter().map(|image| image.levels.iter().sum::<u64>()).sum::<u64>() > mib(256));
    }

    #[test]
    fn a_miss_uses_the_coarser_mip_or_the_factor() {
        let factor = [0.2, 0.0, 0.0, 1.0];
        let resident = [None, Some([1.0, 0.0, 0.0, 1.0])];
        assert_eq!(miss_colour(&resident, 0, factor), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(miss_colour(&[None, None], 0, factor), factor);
        assert_ne!(miss_colour(&[None, None], 0, factor), [0.0, 0.0, 0.0, 1.0]);
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
