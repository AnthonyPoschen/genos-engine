//! One byte budget for resident shape blocks and particle images.
//!
//! A draw admits the shapes and the particle image for that frame. Required
//! in-view shapes outrank unused blocks and optional particle images. When the
//! total would pass the budget, unused blocks drop first. The particle image
//! then shrinks or drops. A shape that fits on its own stays.

use crate::world::ParticleFrame;

const DEFAULT_BUDGET: u64 = 64 * 1024 * 1024;

struct ShapeSlot {
    key: u64,
    bytes: u64,
    used: bool,
}

/// What one particle-image admission did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageAdmit {
    pub resident: bool,
    pub bytes: u64,
    /// The pixel buffer was copied or scaled on this call.
    pub rebuilt: bool,
    /// The GPU image needs a new upload.
    pub uploaded: bool,
}

/// Ledger the draw uses for shape blocks and the particle image.
pub struct FrameMemory {
    limit: u64,
    shapes: Vec<ShapeSlot>,
    dropped: Vec<u64>,
    image_key: u64,
    image_width: u32,
    image_height: u32,
    image_pixels: Vec<u8>,
    image_bytes: u64,
    image_ready: bool,
    image_used: bool,
    last_rebuilt: bool,
    last_uploaded: bool,
    frames: Vec<ParticleFrame>,
}

impl Default for FrameMemory {
    fn default() -> Self {
        Self::with_limit(DEFAULT_BUDGET)
    }
}

impl FrameMemory {
    pub fn with_limit(limit: u64) -> Self {
        Self {
            limit,
            shapes: Vec::new(),
            dropped: Vec::new(),
            image_key: 0,
            image_width: 0,
            image_height: 0,
            image_pixels: Vec::new(),
            image_bytes: 0,
            image_ready: false,
            image_used: false,
            last_rebuilt: false,
            last_uploaded: false,
            frames: Vec::new(),
        }
    }

    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Accounted resident bytes. Unused blocks still count until they drop.
    pub fn accounted(&self) -> u64 {
        let shapes = self.shapes.iter().map(|slot| slot.bytes).sum::<u64>();
        shapes + self.image_resident_bytes()
    }

    pub fn begin_frame(&mut self) {
        for slot in &mut self.shapes {
            slot.used = false;
        }
        self.image_used = false;
        self.dropped.clear();
        self.frames.clear();
        self.last_rebuilt = false;
        self.last_uploaded = false;
    }

    pub fn shape_bytes(&self, key: u64) -> Option<u64> {
        self.shapes
            .iter()
            .find(|slot| slot.key == key)
            .map(|slot| slot.bytes)
    }

    pub fn contains_shape(&self, key: u64) -> bool {
        self.shapes.iter().any(|slot| slot.key == key)
    }

    /// Admit every shape for this frame. A key that is already resident stays.
    /// A new key drops lower-priority residents until it fits.
    pub fn admit_shapes(&mut self, needed: &[(u64, u64)]) {
        let mut fresh = Vec::new();
        for &(key, bytes) in needed {
            if bytes == 0 {
                continue;
            }
            if let Some(slot) = self.shapes.iter_mut().find(|slot| slot.key == key) {
                slot.used = true;
                slot.bytes = bytes;
                continue;
            }
            // One block per key. A second instance of the same shape is not a second bill.
            if fresh.iter().any(|(have, _)| *have == key) {
                continue;
            }
            fresh.push((key, bytes));
        }
        for (key, bytes) in fresh {
            self.make_room(bytes);
            if self.accounted().saturating_add(bytes) <= self.limit {
                self.shapes.push(ShapeSlot {
                    key,
                    bytes,
                    used: true,
                });
            }
        }
    }

    pub fn take_dropped(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.dropped)
    }

    /// Drop shapes this frame did not admit. Returns their keys.
    pub fn finish_frame(&mut self) -> Vec<u64> {
        let mut gone = Vec::new();
        self.shapes.retain(|slot| {
            if slot.used {
                true
            } else {
                gone.push(slot.key);
                false
            }
        });
        if !self.image_used {
            self.relieve();
        }
        gone
    }

    pub fn image_matches(&self, key: u64) -> bool {
        self.image_ready && self.image_key == key
    }

    pub fn image_resident(&self) -> bool {
        self.image_ready
    }

    pub fn image_resident_bytes(&self) -> u64 {
        if self.image_ready {
            self.image_bytes
        } else {
            0
        }
    }

    pub fn image_size(&self) -> (u32, u32) {
        if self.image_ready {
            (self.image_width, self.image_height)
        } else {
            (0, 0)
        }
    }

    pub fn image_pixels(&self) -> &[u8] {
        if self.image_ready {
            &self.image_pixels
        } else {
            &[]
        }
    }

    pub fn image_rebuilt(&self) -> bool {
        self.last_rebuilt
    }

    pub fn image_uploaded(&self) -> bool {
        self.last_uploaded
    }

    pub fn clear_image_upload(&mut self) {
        self.last_uploaded = false;
    }

    pub fn clear_frames(&mut self) {
        self.frames.clear();
    }

    pub fn note_frame(&mut self, frame: ParticleFrame) {
        self.frames.push(frame);
    }

    pub fn sampled_frames(&self) -> &[ParticleFrame] {
        &self.frames
    }

    /// Keep a particle image. The same key does not copy pixels again.
    /// A new key copies `pixels` once, then shrinks or drops until the budget holds.
    pub fn admit_image(&mut self, key: u64, width: u32, height: u32, pixels: &[u8]) -> ImageAdmit {
        if self.image_matches(key) {
            self.image_used = true;
            let before = self.image_bytes;
            let ready_before = self.image_ready;
            self.relieve();
            let changed = self.image_bytes != before || self.image_ready != ready_before;
            if changed {
                self.last_rebuilt = true;
                self.last_uploaded = self.image_ready;
            }
            return self.image_admit();
        }
        self.image_key = key;
        self.image_width = width;
        self.image_height = height;
        self.image_pixels = pixels.to_vec();
        self.image_bytes = self.image_pixels.len() as u64;
        self.image_ready = width > 0 && height > 0 && !self.image_pixels.is_empty();
        self.image_used = true;
        self.relieve();
        self.last_rebuilt = true;
        self.last_uploaded = self.image_ready;
        self.image_admit()
    }

    fn image_admit(&self) -> ImageAdmit {
        ImageAdmit {
            resident: self.image_ready,
            bytes: self.image_resident_bytes(),
            rebuilt: self.last_rebuilt,
            uploaded: self.last_uploaded,
        }
    }

    fn make_room(&mut self, extra: u64) {
        let mut guard = 0;
        while self.accounted().saturating_add(extra) > self.limit && guard < 32 {
            guard += 1;
            if self.drop_one_unused_shape() {
                continue;
            }
            if self.shrink_or_drop_image() {
                continue;
            }
            break;
        }
    }

    fn relieve(&mut self) {
        self.make_room(0);
    }

    fn drop_one_unused_shape(&mut self) -> bool {
        let Some(index) = self.shapes.iter().position(|slot| !slot.used) else {
            return false;
        };
        let slot = self.shapes.swap_remove(index);
        self.dropped.push(slot.key);
        true
    }

    fn shrink_or_drop_image(&mut self) -> bool {
        if !self.image_ready {
            return false;
        }
        if self.image_width <= 1 && self.image_height <= 1 {
            self.drop_image();
            return true;
        }
        let width = (self.image_width / 2).max(1);
        let height = (self.image_height / 2).max(1);
        if width == self.image_width && height == self.image_height {
            self.drop_image();
            return true;
        }
        self.image_pixels = downsample(
            &self.image_pixels,
            self.image_width,
            self.image_height,
            width,
            height,
        );
        self.image_width = width;
        self.image_height = height;
        self.image_bytes = self.image_pixels.len() as u64;
        self.last_rebuilt = true;
        self.last_uploaded = true;
        true
    }

    fn drop_image(&mut self) {
        self.image_ready = false;
        self.image_pixels.clear();
        self.image_bytes = 0;
        self.image_width = 0;
        self.image_height = 0;
    }
}

fn downsample(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w * dst_h * 4) as usize];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return out;
    }
    for y in 0..dst_h {
        for x in 0..dst_w {
            let sx = (x * src_w / dst_w).min(src_w - 1);
            let sy = (y * src_h / dst_h).min(src_h - 1);
            let from = ((sy * src_w + sx) * 4) as usize;
            let to = ((y * dst_w + x) * 4) as usize;
            if from + 3 < src.len() && to + 3 < out.len() {
                out[to..to + 4].copy_from_slice(&src[from..from + 4]);
            }
        }
    }
    out
}
