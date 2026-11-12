//! Path-traced indirect irradiance at every receiver pixel: the ground truth
//! the cache is graded against.
//!
//! One sample is a cosine-weighted path from the pixel. Vertex `k` adds
//! `albedo^k * direct(x_k)` (next-event estimation to every lamp). Russian
//! roulette from the third vertex on survives with probability `albedo`, so the
//! weight stays one and the sum is unbiased for all bounces.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::geom::{cosine_dir, EPS, V3};
use crate::rng::Rng;
use crate::scene::Scene;

/// Bumped when the estimator changes so stale disk copies are not reused.
const REF_VERSION: u32 = 1;

/// Indirect irradiance at one point.
#[must_use]
pub fn indirect_sample(scene: &Scene, p: V3, n: V3, rng: &mut Rng) -> f64 {
    let rho = scene.world.albedo;
    let mut o = p.add(n.scale(EPS * 4.0));
    let mut normal = n;
    let mut weight = 1.0;
    let mut sum = 0.0;
    for depth in 1..64 {
        let d = cosine_dir(normal, rng);
        let Some(hit) = scene.world.trace(o, d, 1e9) else {
            break;
        };
        if hit.n.dot(d) > 0.0 {
            break;
        }
        weight *= rho;
        sum += weight * scene.world.direct(hit.p, hit.n, true);
        if depth >= 2 {
            if rng.next_f64() >= rho {
                break;
            }
            weight /= rho;
        }
        o = hit.p.add(hit.n.scale(EPS * 4.0));
        normal = hit.n;
    }
    sum
}

/// Reference indirect irradiance per pixel with the lamps on (invalid pixels 0).
/// `spp` paths per pixel, `threads` workers; the result does not depend on
/// `threads`. Cached under `cache_dir` when given.
#[must_use]
pub fn indirect(
    scene: &Scene,
    spp: usize,
    seed: u64,
    threads: usize,
    cache_dir: Option<&Path>,
) -> Vec<f64> {
    let path = cache_dir.map(|d| cache_path(d, scene, spp, seed));
    if let Some(v) = path.as_deref().and_then(|p| load(p, scene.pixels.len())) {
        return v;
    }
    let n = scene.pixels.len();
    let mut out = vec![0.0; n];
    let chunk = 256;
    let chunks: Vec<(usize, &mut [f64])> = out.chunks_mut(chunk).enumerate().collect();
    let threads = threads.max(1);
    let mut lanes: Vec<Vec<(usize, &mut [f64])>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, c) in chunks {
        lanes[i % threads].push((i, c));
    }
    std::thread::scope(|s| {
        for lane in lanes {
            s.spawn(move || {
                for (ci, slice) in lane {
                    let mut rng = Rng::new(
                        seed ^ (0x9e37_79b9_7f4a_7c15_u64
                            .wrapping_mul(u64::try_from(ci).unwrap_or(0) + 1)),
                    );
                    for (k, v) in slice.iter_mut().enumerate() {
                        let px = &scene.pixels[ci * chunk + k];
                        if !px.valid {
                            continue;
                        }
                        let mut acc = 0.0;
                        for _ in 0..spp {
                            acc += indirect_sample(scene, px.p, px.n, &mut rng);
                        }
                        *v = acc / crate::geom::fl(spp);
                    }
                }
            });
        }
    });
    if let Some(p) = path {
        save(&p, &out);
    }
    out
}

fn cache_path(dir: &Path, scene: &Scene, spp: usize, seed: u64) -> PathBuf {
    let tag = format!(
        "{}-v{REF_VERSION}-px{:.4}-spp{spp}-seed{seed}-n{}.bin",
        scene.name,
        scene.pixel_size,
        scene.pixels.len()
    );
    dir.join(tag)
}

fn load(path: &Path, n: usize) -> Option<Vec<f64>> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() != n * 8 {
        return None;
    }
    Some(
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect(),
    )
}

fn save(path: &Path, v: &[f64]) {
    if let Some(dir) = path.parent() {
        if fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let Ok(mut f) = fs::File::create(path) else {
        return;
    };
    let mut bytes = Vec::with_capacity(v.len() * 8);
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    // A failed write only costs a recompute next run.
    let _ = f.write_all(&bytes);
}
