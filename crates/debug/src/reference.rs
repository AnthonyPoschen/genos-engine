//! Ground truth: a brute-force path trace of the frozen scene from the picture's
//! pose, and the comparison of a live picture against it.
//!
//! The trace uses the same surfaces, lamp unit, paint albedo and tone curve as the
//! picture (genos-render `trace`), with no probes, no near-field shortcut and no
//! bounce limit: every pixel's primary hit gathers next-event direct light at each
//! vertex and continues along cosine-sampled paths, ended by Russian roulette after
//! three bounces (capped at `max_bounces`). It runs on the CPU, so it is exact up to
//! its noise, whatever the GPU path does. Not traced: fog, particle cards, textures.

use std::sync::atomic::{AtomicUsize, Ordering};

use genos_render::{direct_light_in, linear_from_display, SceneMesh, Surfaces, TraceHit};
use genos_scene::{look_direction, Camera, Scene};

use crate::image::{Image, Region};

const LAMBERT: f32 = 0.254647909;
const PI: f32 = std::f32::consts::PI;
const REACH: f32 = 200.0;
/// Vertical field of view of the picture (genos-scene `view_proj`).
const FOV_Y: f32 = 60.0;

/// What to trace.
#[derive(Clone, Debug)]
pub struct RefSetup {
    pub scene: Scene,
    pub eye: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub width: u32,
    pub height: u32,
    /// Paths per pixel per pass.
    pub spp: u32,
    /// Stop when the mean relative standard error of a pixel is this low...
    pub noise_target: f32,
    /// ...or at this many paths per pixel...
    pub max_spp: u32,
    /// ...or after this long (at least one pass runs).
    pub seconds: f32,
    /// Hard cap on path length; Russian roulette ends most paths long before.
    pub max_bounces: u32,
    /// Trace the scene's triangles ([`SceneMesh`], through the CPU BVH) instead of
    /// its analytic shapes. Both must agree within the trace's noise.
    pub triangles: bool,
}

impl RefSetup {
    pub fn from_camera(scene: &Scene, camera: &Camera, width: u32, height: u32, spp: u32) -> Self {
        Self {
            scene: scene.clone(),
            eye: [camera.position.x, camera.position.y, camera.position.z],
            yaw: camera.yaw,
            pitch: camera.pitch,
            width,
            height,
            spp: spp.max(1),
            noise_target: 0.0,
            max_spp: spp.max(1),
            seconds: f32::INFINITY,
            max_bounces: 64,
            triangles: false,
        }
    }
}

/// A finished trace.
#[derive(Clone, Debug)]
pub struct Reference {
    /// Display values, toned like the picture.
    pub image: Image,
    /// Linear radiance per pixel, before the tone curve.
    pub linear: Vec<[f32; 3]>,
    /// False where the primary ray met nothing (left out of comparisons).
    pub hit: Vec<bool>,
    pub spp: u32,
    pub seconds: f32,
    /// Mean standard error of a pixel's luminance over its mean (hit pixels): the
    /// reference's own noise. Comparisons below it mean nothing; raise spp.
    pub noise: f32,
}

fn tone(x: f32) -> f32 {
    if x <= 0.64 {
        x.max(0.0)
    } else {
        let extra = x - 0.64;
        0.64 + 0.14 * (extra / (extra + 1.1))
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
        .sqrt()
        .max(1.0e-12);
    scale(a, 1.0 / l)
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> f32 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn cosine(n: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let r = u.sqrt();
    let phi = 2.0 * PI * v;
    let up = if n[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    normalize(add(
        add(scale(t, r * phi.cos()), scale(b, r * phi.sin())),
        scale(n, (1.0 - u).max(0.0).sqrt()),
    ))
}

fn lift(hit: &TraceHit) -> [f32; 3] {
    add(hit.point, scale(hit.normal, 0.02))
}

/// Light leaving the primary hit toward the eye, one path.
#[cfg(test)]
fn path(scene: &Scene, first: &TraceHit, max_bounces: u32, rng: &mut Rng) -> [f32; 3] {
    let u = (rng.next(), rng.next());
    path_from(scene, scene, first, max_bounces, u, rng)
}

/// One path whose first bounce direction comes from `stratum`, a cell of the pixel's
/// sample grid (less noise than chance).
fn path_from(
    scene: &Scene,
    surfaces: &dyn Surfaces,
    first: &TraceHit,
    max_bounces: u32,
    stratum: (f32, f32),
    rng: &mut Rng,
) -> [f32; 3] {
    let sky = scene.sky.as_ref().map_or([0.0; 3], |s| s.color);
    let mut at = *first;
    let mut weight = scale(first.albedo, LAMBERT);
    let mut sum = mul(weight, direct_light_in(scene, surfaces, lift(&at), at.normal));
    for bounce in 0..max_bounces {
        let (u, v) = if bounce == 0 {
            stratum
        } else {
            (rng.next(), rng.next())
        };
        let dir = cosine(at.normal, u, v);
        // A cosine sample estimates the cosine-weighted mean radiance; irradiance
        // is π times it.
        weight = scale(weight, PI);
        let Some(next) = surfaces.first_hit(lift(&at), dir, REACH) else {
            sum = add(sum, mul(weight, sky));
            break;
        };
        weight = mul(weight, scale(next.albedo, LAMBERT));
        sum = add(
            sum,
            mul(weight, direct_light_in(scene, surfaces, lift(&next), next.normal)),
        );
        if bounce >= 2 {
            let keep = (next.albedo.iter().fold(0.0f32, |m, c| m.max(*c)) * LAMBERT * PI)
                .clamp(0.05, 0.95);
            if rng.next() >= keep {
                break;
            }
            weight = scale(weight, 1.0 / keep);
        }
        at = next;
    }
    sum
}

/// Trace the whole picture on every core in passes of `spp` paths per pixel until
/// the noise is at most `noise_target`, `max_spp` is reached or `seconds` run out
/// (after at least one pass). Blocks until done.
pub fn render(setup: &RefSetup) -> Reference {
    let started = std::time::Instant::now();
    let (w, h) = (setup.width.max(1) as usize, setup.height.max(1) as usize);
    let forward = {
        let d = look_direction(setup.yaw, setup.pitch);
        [d.x, d.y, d.z]
    };
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let tan_y = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = w as f32 / h as f32;
    let sky = setup.scene.sky.as_ref().map_or([0.0; 3], |s| s.color);
    let mesh;
    let surfaces: &dyn Surfaces = if setup.triangles {
        mesh = SceneMesh::from_scene(&setup.scene);
        &mesh
    } else {
        &setup.scene
    };
    let firsts: Vec<Option<TraceHit>> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let nx = (2.0 * (x as f32 + 0.5) / w as f32 - 1.0) * tan_y * aspect;
            let ny = (1.0 - 2.0 * (y as f32 + 0.5) / h as f32) * tan_y;
            let dir = normalize(add(add(forward, scale(right, nx)), scale(up, ny)));
            surfaces.first_hit(setup.eye, dir, REACH)
        })
        .collect();
    let hit: Vec<bool> = firsts.iter().map(Option::is_some).collect();
    let hits = hit.iter().filter(|h| **h).count();
    // Per pixel: sum of radiance and of squared luminance.
    let mut sum = vec![[0.0f64; 3]; w * h];
    let mut sq = vec![0.0f64; w * h];
    let batch = setup.spp.max(1);
    let side = (batch as f32).sqrt().floor().max(1.0) as u32;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut done = 0u32;
    let mut noise: f32;
    loop {
        let next_row = AtomicUsize::new(0);
        let pass = done / batch;
        let rows: Vec<(usize, Vec<[f64; 3]>, Vec<f64>)> = std::thread::scope(|s| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let y = next_row.fetch_add(1, Ordering::Relaxed);
                            if y >= h {
                                break;
                            }
                            let mut row = vec![[0.0f64; 3]; w];
                            let mut row_sq = vec![0.0f64; w];
                            for x in 0..w {
                                let Some(first) = &firsts[y * w + x] else {
                                    continue;
                                };
                                // First bounces on a jittered grid, shifted per pixel and pass.
                                let mut shift = Rng::new(
                                    ((y as u64) << 32) ^ x as u64 ^ ((pass as u64) << 52) ^ 0x5EED,
                                );
                                let (sx, sy) = (shift.next(), shift.next());
                                for k in 0..batch {
                                    let seed =
                                        ((y as u64) << 40) ^ ((x as u64) << 20) ^ (done + k) as u64;
                                    let mut rng = Rng::new(seed);
                                    let cell = k % (side * side);
                                    let u = ((cell % side) as f32 + rng.next()) / side as f32;
                                    let v = ((cell / side) as f32 + rng.next()) / side as f32;
                                    let stratum = ((u + sx).fract(), (v + sy).fract());
                                    let l = path_from(
                                        &setup.scene,
                                        surfaces,
                                        first,
                                        setup.max_bounces,
                                        stratum,
                                        &mut rng,
                                    );
                                    let y = luma(l) as f64;
                                    row_sq[x] += y * y;
                                    for c in 0..3 {
                                        row[x][c] += l[c] as f64;
                                    }
                                }
                            }
                            out.push((y, row, row_sq));
                        }
                        out
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap_or_default())
                .collect()
        });
        for (y, row, row_sq) in rows {
            for x in 0..w {
                for c in 0..3 {
                    sum[y * w + x][c] += row[x][c];
                }
                sq[y * w + x] += row_sq[x];
            }
        }
        done += batch;
        let n = done as f64;
        let mut total = 0.0f64;
        for i in 0..w * h {
            if hit[i] {
                let mean = (0.2126 * sum[i][0] + 0.7152 * sum[i][1] + 0.0722 * sum[i][2]) / n;
                let var = (sq[i] / n - mean * mean).max(0.0);
                total += (var / n).sqrt() / mean.max(1.0e-4);
            }
        }
        noise = (total / hits.max(1) as f64) as f32;
        let out_of_time = started.elapsed().as_secs_f32() >= setup.seconds;
        if noise <= setup.noise_target || done + batch > setup.max_spp.max(batch) || out_of_time {
            break;
        }
    }
    let linear: Vec<[f32; 3]> = (0..w * h)
        .map(|i| {
            if hit[i] {
                sum[i].map(|v| (v / done as f64) as f32)
            } else {
                sky
            }
        })
        .collect();
    let rgb = linear
        .iter()
        .flat_map(|c| c.map(|v| (tone(v) * 255.0).round().clamp(0.0, 255.0) as u8))
        .collect();
    Reference {
        image: Image {
            width: w as u32,
            height: h as u32,
            rgb,
        },
        linear,
        hit,
        spp: done,
        seconds: started.elapsed().as_secs_f32(),
        noise,
    }
}

/// `render`, reusing a trace saved under `dir` for the same scene, pose, size and
/// bounce limit when it is at least as converged as asked; a new trace is saved there.
/// The key hashes the scene's Debug text, so it holds within one build of the engine.
pub fn render_cached(setup: &RefSetup, dir: &std::path::Path) -> Reference {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!(
        "{:?}|{:?}|{}|{}|{}|{}|{}",
        setup.scene,
        setup.eye,
        setup.yaw,
        setup.pitch,
        setup.width,
        setup.height,
        setup.max_bounces
    )
    .hash(&mut hasher);
    // Analytic traces keep their old keys, so existing caches stay valid.
    if setup.triangles {
        "triangles".hash(&mut hasher);
    }
    let path = dir.join(format!("{:016x}.ref", hasher.finish()));
    if let Some(r) = load(&path, setup.width, setup.height) {
        if r.noise <= setup.noise_target || r.spp >= setup.max_spp {
            return r;
        }
    }
    let r = render(setup);
    let _ = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, save(&r)));
    r
}

fn save(r: &Reference) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + r.linear.len() * 13);
    for v in [r.image.width, r.image.height, r.spp] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&r.noise.to_le_bytes());
    for (c, h) in r.linear.iter().zip(&r.hit) {
        for v in c {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(*h as u8);
    }
    out
}

fn load(path: &std::path::Path, width: u32, height: u32) -> Option<Reference> {
    let bytes = std::fs::read(path).ok()?;
    let word = |i: usize| {
        bytes
            .get(i * 4..i * 4 + 4)
            .map(|b| [b[0], b[1], b[2], b[3]])
    };
    let (w, h, spp) = (
        u32::from_le_bytes(word(0)?),
        u32::from_le_bytes(word(1)?),
        u32::from_le_bytes(word(2)?),
    );
    let noise = f32::from_le_bytes(word(3)?);
    let n = (w * h) as usize;
    if (w, h) != (width, height) || bytes.len() != 16 + n * 13 {
        return None;
    }
    let mut linear = Vec::with_capacity(n);
    let mut hit = Vec::with_capacity(n);
    for i in 0..n {
        let at = 16 + i * 13;
        let f = |k: usize| {
            f32::from_le_bytes([
                bytes[at + k],
                bytes[at + k + 1],
                bytes[at + k + 2],
                bytes[at + k + 3],
            ])
        };
        linear.push([f(0), f(4), f(8)]);
        hit.push(bytes[at + 12] != 0);
    }
    let rgb = linear
        .iter()
        .flat_map(|c| c.map(|v| (tone(v) * 255.0).round().clamp(0.0, 255.0) as u8))
        .collect();
    Some(Reference {
        image: Image {
            width: w,
            height: h,
            rgb,
        },
        linear,
        hit,
        spp,
        seconds: 0.0,
        noise,
    })
}

/// Live against reference, in linear luminance (the tone curve undone).
#[derive(Clone, Debug, PartialEq)]
pub struct Compare {
    /// Blocks compared (at least half their pixels met a surface).
    pub pixels: usize,
    pub mean_abs: f32,
    /// Sum of |live - ref| over the sum of ref: the share of the light that is wrong.
    pub mean_rel: f32,
    /// 95th percentile of |live - ref|.
    pub p95_abs: f32,
    /// 95th percentile of |live - ref| / max(ref, 0.02).
    pub p95_rel: f32,
    /// (sum live - sum ref) / sum ref: above 0 the picture is too bright.
    pub bias: f32,
    pub ref_mean: f32,
    pub live_mean: f32,
    /// Per grid cell, row-major: (mean_rel, bias).
    pub regions: Vec<(f32, f32)>,
    pub grid: (u32, u32),
}

/// Linear luminance of a displayed pixel, the tone curve undone.
fn display_y(p: &[u8]) -> f32 {
    let c = [0, 1, 2].map(|i| linear_from_display(p[i] as f32 / 255.0));
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Compare `live` (resampled to the reference size) with `reference` over `region`,
/// on `block` x `block` pixel averages (the reference's noise falls with the block
/// side), with stats per `grid` cell. The heatmap is red where the picture is too
/// bright, blue where too dark, full at 50 % relative error; grey where nothing was
/// compared.
pub fn compare(
    live: &Image,
    reference: &Reference,
    region: Region,
    grid: (u32, u32),
    block: u32,
) -> (Compare, Image) {
    let r = &reference.image;
    let (w, h) = (r.width, r.height);
    let live = live.resized(w, h);
    let (x0, y0, x1, y1) = region.pixels(w, h);
    let (gx, gy) = (grid.0.max(1), grid.1.max(1));
    let block = block.max(1);
    let mut cells = vec![(0.0f64, 0.0f64, 0.0f64); (gx * gy) as usize];
    let mut abs = Vec::new();
    let mut rel = Vec::new();
    let (mut sl, mut sr, mut sa, mut sw) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut heat = vec![40u8; (w * h * 3) as usize];
    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        while bx < x1 {
            let (ex, ey) = ((bx + block).min(x1), (by + block).min(y1));
            let (mut ly, mut ry, mut n) = (0.0f32, 0.0f32, 0u32);
            for y in by..ey {
                for x in bx..ex {
                    let i = (y * w + x) as usize;
                    if reference.hit[i] {
                        ly += display_y(&live.rgb[i * 3..i * 3 + 3]);
                        ry += display_y(&r.rgb[i * 3..i * 3 + 3]);
                        n += 1;
                    }
                }
            }
            if n * 2 >= (ex - bx) * (ey - by) {
                let (ly, ry) = (ly / n as f32, ry / n as f32);
                let d = ly - ry;
                let weight = n as f64;
                sl += ly as f64 * weight;
                sr += ry as f64 * weight;
                sa += d.abs() as f64 * weight;
                sw += weight;
                abs.push(d.abs());
                let e = d / ry.max(0.02);
                rel.push(e.abs());
                let cx = ((bx - x0) * gx / (x1 - x0).max(1)).min(gx - 1);
                let cy = ((by - y0) * gy / (y1 - y0).max(1)).min(gy - 1);
                let c = &mut cells[(cy * gx + cx) as usize];
                c.0 += d.abs() as f64 * weight;
                c.1 += d as f64 * weight;
                c.2 += ry as f64 * weight;
                let t = (e.abs() / 0.5).min(1.0);
                let base = ry.sqrt().min(1.0) * 60.0;
                let color = if d > 0.0 {
                    [(base + t * (255.0 - base)) as u8, base as u8, base as u8]
                } else {
                    [
                        base as u8,
                        (base + t * 0.3 * (255.0 - base)) as u8,
                        (base + t * (255.0 - base)) as u8,
                    ]
                };
                for y in by..ey {
                    for x in bx..ex {
                        let i = (y * w + x) as usize;
                        heat[i * 3..i * 3 + 3].copy_from_slice(&color);
                    }
                }
            }
            bx += block;
        }
        by += block;
    }
    let pct = |v: &mut Vec<f32>| {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * 0.95) as usize]
    };
    let n = abs.len();
    let compared = sw.max(1.0);
    let denom = sr.max(1.0e-9);
    (
        Compare {
            pixels: n,
            mean_abs: (sa / compared) as f32,
            mean_rel: (sa / denom) as f32,
            p95_abs: pct(&mut abs),
            p95_rel: pct(&mut rel),
            bias: ((sl - sr) / denom) as f32,
            ref_mean: (sr / compared) as f32,
            live_mean: (sl / compared) as f32,
            regions: cells
                .iter()
                .map(|c| {
                    (
                        (c.0 / c.2.max(1.0e-9)) as f32,
                        (c.1 / c.2.max(1.0e-9)) as f32,
                    )
                })
                .collect(),
            grid: (gx, gy),
        },
        Image {
            width: w,
            height: h,
            rgb: heat,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_scene::{Ceiling, Floor, Light, Sky, Vec3, Wall};
    use genos_render::trace_ray;

    fn lamp(x: f32, y: f32, z: f32) -> Light {
        Light {
            position: Vec3::new(x, y, z),
            color: [1.0, 1.0, 1.0],
            direction: Vec3::ZERO,
        }
    }

    fn wall(x: f32, z: f32, half_x: f32, half_z: f32, color: [f32; 3]) -> Wall {
        Wall {
            position: Vec3::new(x, 0.0, z),
            half_x,
            half_z,
            height: 3.0,
            base: 0.0,
            color,
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }
    }

    fn open(lights: Vec<Light>, walls: Vec<Wall>, sky: Option<Sky>) -> Scene {
        Scene {
            floor: Floor {
                position: Vec3::ZERO,
                half_x: 40.0,
                half_z: 40.0,
                color: [1.0; 3],
            },
            walls,
            solids: Vec::new(),
            lights,
            ceiling: None,
            sky,
        }
    }

    /// A shut white 6 x 3 x 6 m room around the origin with one lamp.
    fn closed_room() -> Scene {
        let w = [1.0; 3];
        Scene {
            floor: Floor {
                position: Vec3::ZERO,
                half_x: 3.5,
                half_z: 3.5,
                color: w,
            },
            walls: vec![
                wall(0.0, -3.25, 3.5, 0.25, w),
                wall(0.0, 3.25, 3.5, 0.25, w),
                wall(-3.25, 0.0, 0.25, 3.5, w),
                wall(3.25, 0.0, 0.25, 3.5, w),
            ],
            solids: Vec::new(),
            lights: vec![lamp(0.7, 1.9, -0.4)],
            ceiling: Some(Ceiling {
                height: 3.0,
                color: w,
            }),
            sky: None,
        }
    }

    fn hit_at(scene: &Scene, point: [f32; 3], normal: [f32; 3]) -> TraceHit {
        let from = add(point, scale(normal, 0.5));
        trace_ray(scene, from, scale(normal, -1.0), 2.0).expect("surface under the probe point")
    }

    fn mean_path(scene: &Scene, at: &TraceHit, bounces: u32, n: u32) -> [f32; 3] {
        let mut sum = [0.0; 3];
        for k in 0..n {
            sum = add(
                sum,
                path(scene, at, bounces, &mut Rng::new(k as u64 * 7919 + 1)),
            );
        }
        scale(sum, 1.0 / n as f32)
    }

    fn y(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn a_floor_under_a_white_sky_sends_back_its_albedo() {
        // Furnace: irradiance pi * 1, outgoing albedo/pi * pi = 0.8 exactly.
        let scene = open(Vec::new(), Vec::new(), Some(Sky { color: [1.0; 3] }));
        let at = hit_at(&scene, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let l = mean_path(&scene, &at, 64, 64);
        assert!((l[1] - LAMBERT * PI).abs() < 1.0e-3, "{l:?}");
    }

    #[test]
    fn a_shut_room_keeps_the_light_it_reflects() {
        // In a closed room of uniform reflectance rho, every bounce lands on a wall,
        // so area-averaged total irradiance is direct / (1 - rho): 5 x at rho 0.8.
        let scene = closed_room();
        let rho = LAMBERT * PI;
        let mut rng = Rng::new(42);
        let (mut direct, mut total) = (0.0f64, 0.0f64);
        // Area-uniform points on the six inner faces (each face is 6 x 6 or 6 x 3).
        for _ in 0..1500 {
            let (a, b) = (rng.next() * 6.0 - 3.0, rng.next());
            let face = (rng.next() * 18.0) as u32; // floor 6, ceiling 6, walls 4 x 1.5
            let (p, n) = match face {
                0..=5 => ([a, 0.0, b * 6.0 - 3.0], [0.0, 1.0, 0.0]),
                6..=11 => ([a, 3.0, b * 6.0 - 3.0], [0.0, -1.0, 0.0]),
                12..=13 => ([a, b * 3.0, -3.0], [0.0, 0.0, 1.0]),
                14 => ([a, b * 3.0, 3.0], [0.0, 0.0, -1.0]),
                15 => ([-3.0, b * 3.0, a], [1.0, 0.0, 0.0]),
                _ => ([3.0, b * 3.0, a], [-1.0, 0.0, 0.0]),
            };
            let at = hit_at(&scene, p, n);
            direct += y(path(&scene, &at, 0, &mut rng)) as f64;
            total += y(mean_path(&scene, &at, 256, 8)) as f64;
        }
        let gain = total / direct;
        let want = 1.0 / (1.0 - rho as f64);
        assert!(
            (gain - want).abs() < want * 0.08,
            "gain {gain:.3}, want {want:.3}"
        );
    }

    #[test]
    fn a_lamp_falls_off_with_the_square_of_distance() {
        let near = open(vec![lamp(0.0, 1.0, 0.0)], Vec::new(), None);
        let far = open(vec![lamp(0.0, 2.0, 0.0)], Vec::new(), None);
        let at = hit_at(&near, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        // Nothing for the floor to bounce off: the full trace is the direct light.
        let ratio = y(mean_path(&near, &at, 64, 16)) / y(mean_path(&far, &at, 64, 16));
        // The shaded point is lifted 0.02 m off the floor.
        let want = (1.98f32 / 0.98).powi(2);
        assert!((ratio - want).abs() < 0.02, "{ratio}");
        let side = hit_at(&near, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        // One metre aside, from 0.02 m up: cos 0.70 x (0.96 / 1.96) of the point below.
        let r = y(mean_path(&near, &side, 64, 16)) / y(mean_path(&near, &at, 64, 16));
        assert!((r - 0.3429).abs() < 0.005, "{r}");
    }

    #[test]
    fn a_red_wall_bleeds_onto_a_white_floor() {
        let scene = open(
            vec![lamp(0.0, 1.5, -1.2)],
            vec![wall(0.0, 0.0, 3.0, 0.1, [1.0, 0.1, 0.1])],
            None,
        );
        // Behind the wall from the lamp: only the wall's red bounce gets there? No:
        // in front, beside the lamp, the floor gets white direct plus red bounce.
        let at = hit_at(&scene, [0.0, 0.0, -0.4], [0.0, 1.0, 0.0]);
        let direct = path(&scene, &at, 0, &mut Rng::new(1));
        let total = mean_path(&scene, &at, 64, 2000);
        let bounce = [
            total[0] - direct[0],
            total[1] - direct[1],
            total[2] - direct[2],
        ];
        assert!(
            bounce[0] > 0.0 && bounce[0] > bounce[1] * 3.0,
            "bounce {bounce:?}"
        );
        assert!(
            (direct[0] - direct[1]).abs() < 1.0e-6,
            "direct is white {direct:?}"
        );
    }

    /// Known scenes rendered to PNGs to look at: `GENOS_REFERENCE_OUT=dir cargo test
    /// -p genos-debug --release known_scenes -- --ignored`.
    #[test]
    #[ignore]
    fn known_scenes() {
        let dir = std::path::PathBuf::from(
            std::env::var("GENOS_REFERENCE_OUT").unwrap_or("target/reference-known".into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let mut cornell = closed_room();
        cornell.walls[2].color = [0.9, 0.1, 0.1];
        cornell.walls[3].color = [0.1, 0.8, 0.1];
        cornell.lights = vec![Light {
            color: [0.08; 3],
            ..lamp(0.0, 2.2, 0.0)
        }];
        cornell.solids.push(genos_scene::Solid {
            yaw: 0.4,
            shape: genos_scene::Shape::Square,
            position: Vec3::new(0.8, 0.0, -0.8),
            size: 1.0,
            height: 1.6,
            color: [1.0; 3],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        });
        let mut sunny = open(
            Vec::new(),
            vec![wall(0.0, 0.0, 2.0, 0.1, [0.9, 0.1, 0.1])],
            Some(Sky {
                color: [0.25, 0.35, 0.5],
            }),
        );
        sunny.lights = vec![Light {
            position: Vec3::ZERO,
            color: [1.0, 0.95, 0.85],
            direction: Vec3::new(0.4, -0.7, 0.5),
        }];
        let scenes = [
            ("cornell", cornell, [0.0, 1.5, 2.9], 0.0, -0.1),
            (
                "lamp-floor",
                open(
                    vec![Light {
                        color: [0.05; 3],
                        ..lamp(0.0, 1.0, 0.0)
                    }],
                    Vec::new(),
                    None,
                ),
                [0.0, 2.5, 4.0],
                0.0,
                -0.5,
            ),
            ("sun-wall", sunny, [3.0, 1.6, 5.0], -0.5, -0.15),
        ];
        for (name, scene, eye, yaw, pitch) in scenes {
            for (spp, bounces) in [(256, 64), (256, 0)] {
                let r = render(&RefSetup {
                    scene: scene.clone(),
                    eye,
                    yaw,
                    pitch,
                    width: 240,
                    height: 180,
                    spp,
                    noise_target: 0.0,
                    max_spp: spp,
                    seconds: f32::INFINITY,
                    max_bounces: bounces,
                    triangles: false,
                });
                r.image
                    .save(&dir.join(format!("{name}-b{bounces}.png")))
                    .unwrap();
                eprintln!("{name} b{bounces}: {:.2}s noise {:.4}", r.seconds, r.noise);
            }
        }
    }

    /// The triangle form of the scene (GI v2 phase 1) traces the same picture as the
    /// analytic shapes, within the trace's own noise.
    #[test]
    fn triangles_match_the_analytic_reference() {
        let mut scene = closed_room();
        let solid = |shape, x: f32, z: f32| genos_scene::Solid {
            shape,
            position: Vec3::new(x, 0.0, z),
            size: 0.9,
            height: 1.2,
            yaw: 0.5,
            color: [0.9, 0.3, 0.2],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        };
        scene.solids = vec![
            solid(genos_scene::Shape::Square, -1.2, -1.0),
            solid(genos_scene::Shape::Circle, 1.2, 0.8),
        ];
        let setup = |triangles| RefSetup {
            scene: scene.clone(),
            eye: [0.0, 1.6, 2.7],
            yaw: 0.0,
            pitch: -0.25,
            width: 64,
            height: 48,
            spp: 64,
            noise_target: 0.0,
            max_spp: 64,
            seconds: f32::INFINITY,
            max_bounces: 64,
            triangles,
        };
        let a = render(&setup(false));
        let b = render(&setup(true));
        let (mut diff, mut sum, mut hit_diff) = (0.0f64, 0.0f64, 0);
        for i in 0..a.linear.len() {
            if a.hit[i] != b.hit[i] {
                hit_diff += 1;
                continue;
            }
            if a.hit[i] {
                diff += (luma(a.linear[i]) - luma(b.linear[i])).abs() as f64;
                sum += luma(a.linear[i]) as f64;
            }
        }
        let rel = (diff / sum) as f32;
        // Two independent traces of the same picture differ by about the noise.
        let noise = a.noise.max(b.noise);
        assert_eq!(hit_diff, 0);
        assert!(rel < 2.0 * noise + 0.01, "mean relative difference {rel}, noise {noise}");
        let mean = |r: &Reference| r.linear.iter().map(|c| luma(*c) as f64).sum::<f64>();
        let bias = (mean(&b) / mean(&a) - 1.0).abs() as f32;
        assert!(bias < 0.01, "picture mean differs by {bias}");
    }

    #[test]
    fn a_picture_equal_to_its_reference_has_no_error() {
        let reference = Reference {
            image: Image {
                width: 2,
                height: 1,
                rgb: vec![100, 100, 100, 20, 20, 20],
            },
            linear: vec![[0.4; 3], [0.08; 3]],
            hit: vec![true, true],
            spp: 1,
            seconds: 0.0,
            noise: 0.0,
        };
        let (c, _) = compare(
            &reference.image.clone(),
            &reference,
            Region::default(),
            (2, 1),
            1,
        );
        assert_eq!(c.mean_abs, 0.0);
        assert_eq!(c.bias, 0.0);
        let brighter = Image {
            width: 2,
            height: 1,
            rgb: vec![120, 120, 120, 20, 20, 20],
        };
        let (c, _) = compare(&brighter, &reference, Region::default(), (2, 1), 1);
        assert!(c.bias > 0.0 && c.regions[0].1 > 0.0 && c.regions[1].1 == 0.0);
    }
}
