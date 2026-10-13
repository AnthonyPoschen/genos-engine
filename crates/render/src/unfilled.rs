//! Unfilled-on-screen events: pixels whose surface-cache texel had no finished
//! set when compose showed them.
//!
//! The pictures are downscaled PNGs. This tree has no JPEG encoder, and ADR 0007
//! keeps image codecs in-repo, so a smaller PNG is the record.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Longest edge of a saved picture. Full frames stay on the GPU; the folder keeps this.
pub const UNSCALED_EDGE: u32 = 480;

/// Default session cap so one run cannot fill the disk even before the global caps.
pub const DEFAULT_SESSION_CAP: u32 = 32;

/// What the compose pass counts, and what a hotkey or the panel can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnfilledControl {
    /// Count and print. The tint and the dumps stay off until asked.
    pub count: bool,
    /// Magenta tint on unfilled pixels.
    pub tint: bool,
    /// Save an event when the unfilled share of shaded pixels exceeds this percent.
    pub dump_pct: Option<f32>,
}

impl Default for UnfilledControl {
    fn default() -> Self {
        Self::from_env()
    }
}

impl UnfilledControl {
    pub fn from_env() -> Self {
        let count = env_flag("GENOS_LIGHTING_UNFILLED");
        let tint = env_flag("GENOS_LIGHTING_UNFILLED_TINT");
        let dump_pct = std::env::var("GENOS_LIGHTING_UNFILLED_DUMP")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|n| n.is_finite() && *n >= 0.0);
        Self {
            count: count || tint || dump_pct.is_some(),
            tint,
            dump_pct,
        }
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("on")
    )
}

/// One frame's counters. `pct` is the percent of shaded pixels that were unfilled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnfilledStats {
    pub frame: u32,
    pub pixels: u32,
    pub shaded: u32,
    pub max_run: u32,
    pub pct: f32,
}

impl UnfilledStats {
    pub fn from_counts(frame: u32, pixels: u32, shaded: u32, max_run: u32) -> Self {
        let pct = if shaded == 0 {
            0.0
        } else {
            100.0 * pixels as f32 / shaded as f32
        };
        Self {
            frame,
            pixels,
            shaded,
            max_run,
            pct,
        }
    }

    pub fn line(self) -> String {
        format!(
            "UNFILLED px={} pct={:.2} max_run={}",
            self.pixels, self.pct, self.max_run
        )
    }
}

/// Disk caps for `~/genos-perf/unfilled`. Age is checked as well as count and size.
#[derive(Clone, Copy, Debug)]
pub struct UnfilledLimits {
    pub max_folders: usize,
    pub max_bytes: u64,
    pub max_age: Duration,
    pub session_cap: u32,
}

impl UnfilledLimits {
    pub fn standard() -> Self {
        Self {
            max_folders: 200,
            max_bytes: 500 * 1024 * 1024,
            max_age: Duration::from_secs(2 * 24 * 60 * 60),
            session_cap: DEFAULT_SESSION_CAP,
        }
    }

    pub fn from_env() -> Self {
        let base = Self::standard();
        Self {
            max_folders: env_usize("GENOS_UNFILLED_MAX_FOLDERS", base.max_folders),
            max_bytes: env_u64("GENOS_UNFILLED_MAX_BYTES", base.max_bytes),
            max_age: Duration::from_secs(env_u64(
                "GENOS_UNFILLED_MAX_AGE_SECS",
                base.max_age.as_secs(),
            )),
            session_cap: env_u64("GENOS_UNFILLED_SESSION_CAP", base.session_cap as u64) as u32,
        }
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `GENOS_UNFILLED_DIR`, or `$HOME/genos-perf/unfilled`.
pub fn unfilled_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GENOS_UNFILLED_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("genos-perf").join("unfilled"))
}

/// Drop aged event folders, then the oldest, until both caps hold.
/// Called at startup and before and after each save.
pub fn rotate_unfilled(
    root: &Path,
    now_ms: u64,
    limits: &UnfilledLimits,
) -> std::io::Result<usize> {
    if !root.is_dir() {
        return Ok(0);
    }
    let mut events = list_events(root)?;
    let mut removed = 0usize;
    let max_age_ms = limits.max_age.as_millis() as u64;
    events.retain(|event| {
        let aged = now_ms.saturating_sub(event.ms) > max_age_ms;
        if aged {
            removed += usize::from(std::fs::remove_dir_all(&event.path).is_ok());
        }
        !aged
    });
    events.sort_by_key(|event| event.ms);
    while over_cap(&events, limits) {
        // A single event that is over the byte cap on its own is the record just
        // written. Removing it would leave the folder empty.
        if events.len() == 1 && events.len() <= limits.max_folders {
            break;
        }
        let Some(oldest) = events.first() else {
            break;
        };
        if std::fs::remove_dir_all(&oldest.path).is_err() {
            break;
        }
        events.remove(0);
        removed += 1;
    }
    Ok(removed)
}

fn over_cap(events: &[EventDir], limits: &UnfilledLimits) -> bool {
    if events.is_empty() {
        return false;
    }
    let bytes: u64 = events.iter().map(|event| event.bytes).sum();
    events.len() > limits.max_folders || bytes > limits.max_bytes
}

struct EventDir {
    ms: u64,
    path: PathBuf,
    bytes: u64,
}

fn list_events(root: &Path) -> std::io::Result<Vec<EventDir>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type().map(|k| k.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.bytes().all(|b| b.is_ascii_digit()) || name.is_empty() {
            continue;
        }
        let Ok(ms) = name.parse::<u64>() else {
            continue;
        };
        out.push(EventDir {
            ms,
            path: entry.path(),
            bytes: dir_bytes(&entry.path()),
        });
    }
    Ok(out)
}

fn dir_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Startup pass. Missing directories are left alone.
pub fn enforce_unfilled_cap() {
    let Some(root) = unfilled_root() else {
        return;
    };
    let _ = rotate_unfilled(&root, now_ms(), &UnfilledLimits::from_env());
}

pub struct UnfilledDump {
    pub stats: UnfilledStats,
    pub width: u32,
    pub height: u32,
    pub eye: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// BGRA of the picture, tightly packed.
    pub shot: Vec<u8>,
    /// Bit per pixel, 32 pixels per word, row-major. Empty skips the overlay.
    pub mask: Vec<u32>,
}

/// Write one event if this session is under its cap. Rotates before and after.
/// `session_saved` is how many this process has already kept.
pub fn save_unfilled_event(session_saved: u32, dump: &UnfilledDump) -> Result<bool, String> {
    let Some(root) = unfilled_root() else {
        return Ok(false);
    };
    let limits = UnfilledLimits::from_env();
    if session_saved >= limits.session_cap {
        let _ = rotate_unfilled(&root, now_ms(), &limits);
        return Ok(false);
    }
    std::fs::create_dir_all(&root).map_err(|err| err.to_string())?;
    let stamp = now_ms();
    let _ = rotate_unfilled(&root, stamp, &limits);
    let dir = unique_event_dir(&root, stamp);
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let (sw, sh, shot) = downscale_bgra(dump.width, dump.height, &dump.shot, UNSCALED_EDGE);
    write_png(&dir.join("shot.png"), sw, sh, &shot)?;
    if !dump.mask.is_empty() {
        let overlay = overlay_bgra(dump.width, dump.height, &dump.mask);
        let (ow, oh, overlay) = downscale_bgra(dump.width, dump.height, &overlay, UNSCALED_EDGE);
        write_png(&dir.join("overlay.png"), ow, oh, &overlay)?;
    }
    let counts = format!(
        "{{\n  \"frame\": {},\n  \"pixels\": {},\n  \"shaded\": {},\n  \"pct\": {:.4},\n  \"max_run\": {},\n  \"width\": {},\n  \"height\": {}\n}}\n",
        dump.stats.frame,
        dump.stats.pixels,
        dump.stats.shaded,
        dump.stats.pct,
        dump.stats.max_run,
        dump.width,
        dump.height
    );
    std::fs::write(dir.join("counts.json"), counts).map_err(|err| err.to_string())?;
    let camera = format!(
        "{{\n  \"x\": {:.6},\n  \"y\": {:.6},\n  \"z\": {:.6},\n  \"yaw\": {:.6},\n  \"pitch\": {:.6}\n}}\n",
        dump.eye[0], dump.eye[1], dump.eye[2], dump.yaw, dump.pitch
    );
    std::fs::write(dir.join("camera.json"), camera).map_err(|err| err.to_string())?;
    std::fs::write(dir.join("frame.txt"), format!("{}\n", dump.stats.frame))
        .map_err(|err| err.to_string())?;
    let _ = rotate_unfilled(&root, stamp, &limits);
    Ok(true)
}

fn unique_event_dir(root: &Path, stamp: u64) -> PathBuf {
    let mut ms = stamp;
    loop {
        let dir = root.join(format!("{ms:013}"));
        if !dir.exists() {
            return dir;
        }
        ms = ms.saturating_add(1);
    }
}

/// Longest run of set bits along a row or a column.
pub fn max_unfilled_run(width: u32, height: u32, mask: &[u32]) -> u32 {
    if width == 0 || height == 0 || mask.is_empty() {
        return 0;
    }
    let stride = ((width + 31) / 32) as usize;
    let bit = |x: u32, y: u32| -> bool {
        let index = y as usize * stride + (x as usize >> 5);
        mask.get(index)
            .is_some_and(|word| (word >> (x & 31)) & 1 == 1)
    };
    let mut best = 0u32;
    for y in 0..height {
        let mut run = 0u32;
        for x in 0..width {
            if bit(x, y) {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
    }
    for x in 0..width {
        let mut run = 0u32;
        for y in 0..height {
            if bit(x, y) {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
    }
    best
}

pub fn downscale_bgra(width: u32, height: u32, bgra: &[u8], max_edge: u32) -> (u32, u32, Vec<u8>) {
    let edge = width.max(height).max(1);
    if max_edge == 0 || edge <= max_edge || width == 0 || height == 0 {
        return (width, height, bgra.to_vec());
    }
    let scale = edge as f32 / max_edge as f32;
    let nw = ((width as f32 / scale).round() as u32).max(1);
    let nh = ((height as f32 / scale).round() as u32).max(1);
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for y in 0..nh {
        let y0 = (y as u64 * height as u64 / nh as u64) as u32;
        let y1 = (((y as u64 + 1) * height as u64 / nh as u64) as u32).max(y0 + 1);
        for x in 0..nw {
            let x0 = (x as u64 * width as u64 / nw as u64) as u32;
            let x1 = (((x as u64 + 1) * width as u64 / nw as u64) as u32).max(x0 + 1);
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for sy in y0..y1.min(height) {
                for sx in x0..x1.min(width) {
                    let i = ((sy * width + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += bgra.get(i + c).copied().unwrap_or(0) as u64;
                    }
                    n += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            if n == 0 {
                continue;
            }
            for c in 0..4 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    (nw, nh, out)
}

fn overlay_bgra(width: u32, height: u32, mask: &[u32]) -> Vec<u8> {
    let mut out = vec![0u8; width as usize * height as usize * 4];
    let stride = ((width + 31) / 32) as usize;
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * stride + (x as usize >> 5);
            let on = mask
                .get(index)
                .is_some_and(|word| (word >> (x & 31)) & 1 == 1);
            if !on {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            out[i] = 255;
            out[i + 2] = 255;
            out[i + 3] = 255;
        }
    }
    out
}

fn write_png(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    std::fs::write(path, encode_png(width, height, bgra)).map_err(|err| err.to_string())
}

fn encode_png(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let b = bgra.get(i).copied().unwrap_or(0);
            let g = bgra.get(i + 1).copied().unwrap_or(0);
            let r = bgra.get(i + 2).copied().unwrap_or(0);
            raw.extend_from_slice(&[r, g, b]);
        }
    }
    let mut zlib = Vec::new();
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut adler_s1: u32 = 1;
    let mut adler_s2: u32 = 0;
    let mut rest = raw.as_slice();
    while !rest.is_empty() {
        let take = rest.len().min(65535);
        let last = take == rest.len();
        zlib.push(if last { 1 } else { 0 });
        zlib.extend_from_slice(&(take as u16).to_le_bytes());
        zlib.extend_from_slice(&(!take as u16).to_le_bytes());
        zlib.extend_from_slice(&rest[..take]);
        for byte in &rest[..take] {
            adler_s1 = (adler_s1 + u32::from(*byte)) % 65521;
            adler_s2 = (adler_s2 + adler_s1) % 65521;
        }
        rest = &rest[take..];
    }
    zlib.extend_from_slice(&((adler_s2 << 16) | adler_s1).to_be_bytes());
    let mut png = Vec::new();
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut hdr = Vec::new();
    hdr.extend_from_slice(&width.to_be_bytes());
    hdr.extend_from_slice(&height.to_be_bytes());
    hdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &hdr);
    write_chunk(&mut png, b"IDAT", &zlib);
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg() & 0xedb8_8320;
            crc = (crc >> 1) ^ mask;
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(folders: usize, bytes: u64, age: Duration) -> UnfilledLimits {
        UnfilledLimits {
            max_folders: folders,
            max_bytes: bytes,
            max_age: age,
            session_cap: 8,
        }
    }

    fn event(root: &Path, ms: u64, bytes: usize) {
        let dir = root.join(format!("{ms:013}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("counts.json"), vec![b'x'; bytes]).unwrap();
    }

    fn names(root: &Path) -> Vec<u64> {
        let mut ms: Vec<u64> = list_events(root)
            .unwrap()
            .into_iter()
            .map(|event| event.ms)
            .collect();
        ms.sort_unstable();
        ms
    }

    #[test]
    fn unfilled_rotation_applies_age_then_count_and_size() {
        let root = std::env::temp_dir().join(format!("genos-unfilled-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let day: u64 = 24 * 60 * 60 * 1000;
        let now = 10 * day;

        event(&root, now - 3 * day, 10);
        event(&root, now - 2 * day, 10);
        event(&root, now - day, 10);
        event(&root, now - 1000, 10);
        std::fs::create_dir_all(root.join("notes")).unwrap();
        rotate_unfilled(
            &root,
            now,
            &limits(10, 10_000, Duration::from_secs(2 * 24 * 60 * 60)),
        )
        .unwrap();
        assert_eq!(
            names(&root),
            vec![now - 2 * day, now - day, now - 1000],
            "folders older than two days go; exactly two days stays"
        );
        assert!(root.join("notes").is_dir(), "a non-event directory stays");

        let count_root = root.join("count");
        std::fs::create_dir_all(&count_root).unwrap();
        for i in 0..4 {
            event(&count_root, 1_000 + i, 10);
        }
        rotate_unfilled(
            &count_root,
            now,
            &limits(2, 10_000, Duration::from_secs(10 * 24 * 60 * 60)),
        )
        .unwrap();
        assert_eq!(names(&count_root), vec![1_002, 1_003]);

        let size_root = root.join("size");
        std::fs::create_dir_all(&size_root).unwrap();
        event(&size_root, 100, 100);
        event(&size_root, 200, 100);
        event(&size_root, 300, 100);
        rotate_unfilled(
            &size_root,
            now,
            &limits(10, 150, Duration::from_secs(10 * 24 * 60 * 60)),
        )
        .unwrap();
        assert_eq!(
            names(&size_root),
            vec![300],
            "oldest folders go until the byte cap holds"
        );

        let lone_root = root.join("lone");
        std::fs::create_dir_all(&lone_root).unwrap();
        event(&lone_root, 400, 200);
        rotate_unfilled(
            &lone_root,
            now,
            &limits(10, 150, Duration::from_secs(10 * 24 * 60 * 60)),
        )
        .unwrap();
        assert_eq!(
            names(&lone_root),
            vec![400],
            "one event larger than the byte cap is kept"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn max_run_is_the_longer_of_the_row_and_the_column() {
        // 8x4, a horizontal run of 3 and a vertical run of 4.
        let mut mask = vec![0u32; 4];
        mask[0] = 0b0000_0111;
        mask[1] = 0b0000_0001;
        mask[2] = 0b0000_0001;
        mask[3] = 0b0000_0001;
        assert_eq!(max_unfilled_run(8, 4, &mask), 4);
    }
}
