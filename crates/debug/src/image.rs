//! Captured pictures and the measurements the tools take on them.

use std::path::Path;

/// An RGB8 picture, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// A rectangle as fractions of the picture: x, y, width, height in 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region(pub [f32; 4]);

impl Default for Region {
    fn default() -> Self {
        Self([0.0, 0.0, 1.0, 1.0])
    }
}

impl Region {
    /// Pixel bounds (x0, y0, x1, y1), at least one pixel.
    pub fn pixels(self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let [x, y, w, h] = self.0;
        let x0 = ((x.clamp(0.0, 1.0) * width as f32) as u32).min(width.saturating_sub(1));
        let y0 = ((y.clamp(0.0, 1.0) * height as f32) as u32).min(height.saturating_sub(1));
        let x1 =
            (((x + w).clamp(0.0, 1.0) * width as f32).ceil() as u32).clamp(x0 + 1, width.max(1));
        let y1 =
            (((y + h).clamp(0.0, 1.0) * height as f32).ceil() as u32).clamp(y0 + 1, height.max(1));
        (x0, y0, x1, y1)
    }
}

/// Display value 0..255 to linear light.
pub fn linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Rec. 709 luminance of a display pixel, in linear light 0..1.
pub fn pixel_luminance(p: &[u8]) -> f32 {
    0.2126 * linear(p[0]) + 0.7152 * linear(p[1]) + 0.0722 * linear(p[2])
}

impl Image {
    /// From the renderer's BGRA readback.
    pub fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Self {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for px in bgra.chunks_exact(4).take((width * height) as usize) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        rgb.resize((width * height * 3) as usize, 0);
        Self { width, height, rgb }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let source = genos_load::FileSource::new(path);
        let image = genos_load::load_image(&source)
            .map_err(|err| format!("{}: {err:?}", path.display()))?;
        let rgb = image
            .pixels
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        Ok(Self {
            width: image.width,
            height: image.height,
            rgb,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
            }
        }
        std::fs::write(
            path,
            crate::png::encode_rgb(self.width, self.height, &self.rgb),
        )
        .map_err(|err| format!("{}: {err}", path.display()))
    }

    fn at(&self, x: u32, y: u32) -> &[u8] {
        let i = ((y * self.width + x) * 3) as usize;
        &self.rgb[i..i + 3]
    }

    /// Resampled to `width` x `height`: an area average when shrinking, the nearest
    /// pixel when growing.
    pub fn resized(&self, width: u32, height: u32) -> Self {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let width = width.max(1);
        let height = height.max(1);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        for y in 0..height {
            let y0 = (y as f32 * sy) as u32;
            let y1 = (((y + 1) as f32 * sy).ceil() as u32).clamp(y0 + 1, self.height);
            for x in 0..width {
                let x0 = (x as f32 * sx) as u32;
                let x1 = (((x + 1) as f32 * sx).ceil() as u32).clamp(x0 + 1, self.width);
                let mut sum = [0u32; 3];
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let p = self.at(xx, yy);
                        for c in 0..3 {
                            sum[c] += p[c] as u32;
                        }
                    }
                }
                let n = (x1 - x0) * (y1 - y0);
                rgb.extend(sum.map(|s| ((s + n / 2) / n) as u8));
            }
        }
        Self { width, height, rgb }
    }

    /// Mean linear luminance over `region`, and its minimum and maximum.
    pub fn luminance(&self, region: Region) -> Luminance {
        let (x0, y0, x1, y1) = region.pixels(self.width, self.height);
        let mut sum = 0.0f64;
        let mut lo = f32::MAX;
        let mut hi = 0.0f32;
        let mut black = 0u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let l = pixel_luminance(self.at(x, y));
                sum += l as f64;
                lo = lo.min(l);
                hi = hi.max(l);
                black += u32::from(l < 0.002);
            }
        }
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        Luminance {
            mean: (sum / n) as f32,
            min: lo,
            max: hi,
            black_share: (black as f64 / n) as f32,
        }
    }

    /// Compare with `other` (resampled to this size first). The diff picture shows the
    /// absolute difference, scaled up by `gain`.
    pub fn diff(&self, other: &Image, threshold: u8, gain: f32) -> (Diff, Image) {
        let other = other.resized(self.width, self.height);
        let mut out = Vec::with_capacity(self.rgb.len());
        let mut sum = 0u64;
        let mut max = 0u8;
        let mut over = 0u64;
        let mut lum_sum = 0.0f64;
        for (a, b) in self.rgb.chunks_exact(3).zip(other.rgb.chunks_exact(3)) {
            let d = [0, 1, 2].map(|c| a[c].abs_diff(b[c]));
            let worst = d[0].max(d[1]).max(d[2]);
            sum += worst as u64;
            max = max.max(worst);
            over += u64::from(worst > threshold);
            lum_sum += (pixel_luminance(a) - pixel_luminance(b)) as f64;
            out.extend(d.map(|v| (v as f32 * gain).min(255.0) as u8));
        }
        let n = (self.width * self.height).max(1) as f64;
        (
            Diff {
                mean_abs: (sum as f64 / n) as f32,
                max_abs: max,
                over_share: (over as f64 / n) as f32,
                mean_luminance_delta: (lum_sum / n) as f32,
            },
            Image {
                width: self.width,
                height: self.height,
                rgb: out,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Luminance {
    pub mean: f32,
    pub min: f32,
    pub max: f32,
    /// Share of pixels that are black (luminance under 0.002).
    pub black_share: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Diff {
    /// Mean over pixels of the largest channel difference, 0..255.
    pub mean_abs: f32,
    pub max_abs: u8,
    /// Share of pixels whose largest channel difference passes the threshold.
    pub over_share: f32,
    /// Mean luminance of the first minus the second.
    pub mean_luminance_delta: f32,
}

/// Frame-to-frame change over a window of frames, for popping and flicker.
#[derive(Clone, Debug, Default)]
pub struct Flicker {
    region: Region,
    threshold: f32,
    prev: Option<Vec<f32>>,
    /// Per pixel: largest change seen.
    worst: Vec<f32>,
    sum_delta: f64,
    deltas: u64,
    pops: u64,
    means: Vec<f32>,
    size: (u32, u32),
}

/// What [`Flicker`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct FlickerReport {
    pub frames: usize,
    /// Mean per-pixel luminance change between frames.
    pub mean_delta: f32,
    /// Largest per-pixel luminance change between two frames.
    pub max_delta: f32,
    /// Pixel changes larger than the threshold, over all frame pairs.
    pub pops: u64,
    /// Share of pixels that popped at least once.
    pub popped_share: f32,
    /// The region's mean luminance per frame.
    pub means: Vec<f32>,
    /// Largest minus smallest of `means`.
    pub mean_range: f32,
}

impl Flicker {
    /// `threshold` is a luminance change (linear, 0..1) that counts as a pop.
    pub fn new(region: Region, threshold: f32) -> Self {
        Self {
            region,
            threshold,
            ..Self::default()
        }
    }

    pub fn add(&mut self, image: &Image) {
        let (x0, y0, x1, y1) = self.region.pixels(image.width, image.height);
        let size = (x1 - x0, y1 - y0);
        let mut lum = Vec::with_capacity((size.0 * size.1) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                lum.push(pixel_luminance(image.at(x, y)));
            }
        }
        self.means
            .push(lum.iter().map(|v| *v as f64).sum::<f64>() as f32 / lum.len().max(1) as f32);
        if self.size != size {
            self.prev = None;
            self.size = size;
            self.worst = vec![0.0; lum.len()];
        }
        if let Some(prev) = &self.prev {
            for (i, (a, b)) in lum.iter().zip(prev).enumerate() {
                let d = (a - b).abs();
                self.sum_delta += d as f64;
                self.deltas += 1;
                if d > self.threshold {
                    self.pops += 1;
                }
                self.worst[i] = self.worst[i].max(d);
            }
        }
        self.prev = Some(lum);
    }

    pub fn report(&self) -> FlickerReport {
        let max_delta = self.worst.iter().fold(0.0f32, |m, v| m.max(*v));
        let popped = self.worst.iter().filter(|v| **v > self.threshold).count();
        let lo = self.means.iter().fold(f32::MAX, |m, v| m.min(*v));
        let hi = self.means.iter().fold(f32::MIN, |m, v| m.max(*v));
        FlickerReport {
            frames: self.means.len(),
            mean_delta: (self.sum_delta / self.deltas.max(1) as f64) as f32,
            max_delta,
            pops: self.pops,
            popped_share: popped as f32 / self.worst.len().max(1) as f32,
            mean_range: if self.means.is_empty() { 0.0 } else { hi - lo },
            means: self.means.clone(),
        }
    }

    /// Each pixel's largest change, white at `full`.
    pub fn heatmap(&self, full: f32) -> Image {
        let (w, h) = self.size;
        let rgb = self
            .worst
            .iter()
            .flat_map(|v| {
                let t = (v / full.max(1.0e-6)).clamp(0.0, 1.0);
                [
                    (t * 255.0) as u8,
                    (t * t * 255.0) as u8,
                    ((t * 4.0).min(1.0) * 80.0) as u8,
                ]
            })
            .collect();
        Image {
            width: w,
            height: h,
            rgb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: u8) -> Image {
        Image {
            width: w,
            height: h,
            rgb: vec![v; (w * h * 3) as usize],
        }
    }

    #[test]
    fn a_diff_of_one_picture_is_zero() {
        let a = flat(8, 4, 100);
        let (diff, _) = a.diff(&a, 2, 8.0);
        assert_eq!(diff.max_abs, 0);
        assert_eq!(diff.over_share, 0.0);
        let (diff, _) = a.diff(&flat(8, 4, 120), 2, 8.0);
        assert_eq!(diff.max_abs, 20);
        assert_eq!(diff.over_share, 1.0);
    }

    #[test]
    fn a_pop_counts_once_per_pixel_change() {
        let mut f = Flicker::new(Region::default(), 0.05);
        f.add(&flat(4, 4, 50));
        f.add(&flat(4, 4, 50));
        f.add(&flat(4, 4, 200));
        let r = f.report();
        assert_eq!(r.frames, 3);
        assert_eq!(r.pops, 16);
        assert_eq!(r.popped_share, 1.0);
        assert!(r.mean_range > 0.4);
    }

    #[test]
    fn shrinking_averages_and_the_region_is_a_fraction() {
        let mut img = flat(4, 2, 0);
        img.rgb[..6].fill(255);
        let small = img.resized(2, 1);
        assert_eq!(small.rgb[0], 128);
        let l = img.luminance(Region([0.0, 0.0, 0.5, 0.5]));
        assert!((l.mean - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn a_saved_picture_loads_back() {
        let dir = std::env::temp_dir().join(format!("genos-debug-{}", std::process::id()));
        let path = dir.join("a.png");
        let mut img = flat(5, 3, 10);
        img.rgb[7] = 200;
        img.save(&path).unwrap();
        assert_eq!(Image::load(&path).unwrap(), img);
        std::fs::remove_dir_all(dir).ok();
    }
}
