#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_lossless,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_sign_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::many_single_char_names,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::redundant_closure_for_method_calls,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! Sparse signed distance field of one mesh, in object space.
//!
//! The grid's voxel is the mesh's largest side / `voxels_per_side`, clamped. Only
//! 8^3 bricks within the narrow band of the surface are stored, as 8-bit distances
//! (+-1 maps to +-`band`, 4 voxels). A coarse grid with one value per brick (its
//! own band 4 x 8 voxels) covers the whole bounds for empty-space skipping.
//!
//! Sign: a closed mesh (every edge shared by exactly two triangles) is signed by
//! ray parity, the majority of three rays. An open mesh is unsigned (two-sided)
//! and flagged `thin`, so a sheet thinner than a voxel cannot leak through.

use std::collections::HashMap;

use genos_mesh::{Bvh, MeshTriangle};

use crate::{BakeMesh, BakeSettings};

/// Voxels per brick edge.
pub const BRICK: usize = 8;
/// Narrow band in voxels: distances beyond it saturate.
const BAND_VOXELS: f32 = 4.0;
/// Grid cap: 512 voxels (64 bricks) along the largest side.
pub const MAX_VOXELS_PER_SIDE: f32 = 512.0;
/// Voxel of a merged level field ([`build_level`]), metres. A level's meshes
/// each span the building, so the per-mesh rule (largest side / 64, at most
/// 25 cm) gave Sponza 22 to 25 cm voxels: sun shadows a quarter of a metre
/// off and curtains a quarter of a metre thick.
pub const LEVEL_VOXEL: f32 = 0.06;
/// Grid cap of a level field along its largest side. A level longer than
/// 61 m gets a coarser voxel instead of a larger grid.
pub const LEVEL_MAX_VOXELS_PER_SIDE: f32 = 1024.0;

#[derive(Clone, Debug, PartialEq)]
pub struct SdfBrick {
    /// Brick coordinates (in bricks) inside the grid.
    pub at: [u32; 3],
    /// `BRICK^3` encoded distances, x fastest.
    pub distances: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshSdf {
    /// Grid corner in object space.
    pub origin: [f32; 3],
    pub voxel: f32,
    /// Grid size in bricks.
    pub bricks_dims: [u32; 3],
    /// Metres that +-1 (u8 0 and 255) stand for in a brick.
    pub band: f32,
    pub bricks: Vec<SdfBrick>,
    /// One value per brick cell, band `coarse_band`.
    pub coarse: Vec<u8>,
    pub coarse_band: f32,
    /// Open mesh: distances are unsigned and the mesh is treated as two-sided.
    pub thin: bool,
    /// Mean diffuse albedo of the surface in each brick, linear, as bytes.
    /// Parallel to `bricks` in a level field ([`build_level`]). Empty in a
    /// per-mesh field: its instance carries one mean albedo.
    pub brick_albedo: Vec<[u8; 3]>,
}

pub fn encode(d: f32, band: f32) -> u8 {
    (((d / band).clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u8
}

pub fn decode(v: u8, band: f32) -> f32 {
    (v as f32 / 255.0 * 2.0 - 1.0) * band
}

impl MeshSdf {
    /// Distance at `p` (object space) from the nearest voxel: the fine brick when
    /// one is stored there, else the coarse grid (which saturates at its band).
    pub fn distance(&self, p: [f32; 3]) -> f32 {
        let g: [f32; 3] = std::array::from_fn(|a| (p[a] - self.origin[a]) / self.voxel);
        if (0..3).any(|a| g[a] < 0.0 || g[a] >= (self.bricks_dims[a] as usize * BRICK) as f32) {
            return self.coarse_band;
        }
        let v: [usize; 3] = std::array::from_fn(|a| g[a] as usize);
        let b: [u32; 3] = std::array::from_fn(|a| (v[a] / BRICK) as u32);
        if let Some(brick) = self.bricks.iter().find(|k| k.at == b) {
            let l: [usize; 3] = std::array::from_fn(|a| v[a] % BRICK);
            return decode(
                brick.distances[(l[2] * BRICK + l[1]) * BRICK + l[0]],
                self.band,
            );
        }
        let d = self.bricks_dims;
        decode(
            self.coarse[((b[2] * d[1] + b[1]) * d[0] + b[0]) as usize],
            self.coarse_band,
        )
    }

    /// Bytes a GPU pool would hold: bricks plus the coarse grid.
    pub fn bytes(&self) -> usize {
        self.bricks.len() * BRICK * BRICK * BRICK + self.coarse.len()
    }
}

/// Build over a BVH already built on `mesh.triangles`.
pub fn build_with(mesh: &BakeMesh, s: &BakeSettings, bvh: &Bvh) -> MeshSdf {
    let largest = largest_side(mesh);
    // A mesh far larger than a room (a whole level as one mesh) gets coarser voxels
    // instead of an unbounded grid: at most MAX_VOXELS_PER_SIDE along its largest side.
    let voxel = (largest / s.voxels_per_side)
        .clamp(s.voxel_min, s.voxel_max)
        .max(largest / MAX_VOXELS_PER_SIDE);
    build_grid(mesh, voxel, bvh, false)
}

/// The field of a whole static level: `mesh` holds every triangle in level
/// space. The voxel is [`LEVEL_VOXEL`], never the mesh-size rule, and each
/// brick carries the mean albedo of the surface inside it.
pub fn build_level(mesh: &BakeMesh, s: &BakeSettings, bvh: &Bvh) -> MeshSdf {
    let voxel = LEVEL_VOXEL
        .max(s.voxel_min)
        .max(largest_side(mesh) / LEVEL_MAX_VOXELS_PER_SIDE);
    build_grid(mesh, voxel, bvh, true)
}

fn largest_side(mesh: &BakeMesh) -> f32 {
    let bounds = mesh.bounds();
    (0..3).fold(0.0f32, |m, a| m.max((bounds.max[a] - bounds.min[a]).max(0.0)))
}

/// Mean diffuse albedo of each material, linear: a 16 by 16 grid of its
/// textures. A masked material counts only the texels that pass the cutoff.
fn material_albedo(mesh: &BakeMesh) -> Vec<[f32; 3]> {
    const SIDE: u16 = 16;
    (0..mesh.materials.len())
        .map(|m| {
            let cutoff = match mesh.materials[m].pbr.alpha_mode {
                genos_load::AlphaMode::Mask(cutoff) => cutoff,
                _ => 0.0,
            };
            let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let uv = [
                        (f32::from(x) + 0.5) / f32::from(SIDE),
                        (f32::from(y) + 0.5) / f32::from(SIDE),
                    ];
                    let shade = mesh.shade(m as u32, uv);
                    if shade.alpha < cutoff {
                        continue;
                    }
                    let a = shade.diffuse_albedo();
                    for c in 0..3 {
                        sum[c] += a[c];
                    }
                    n += 1.0;
                }
            }
            if n > 0.0 {
                sum.map(|v| v / n)
            } else {
                [0.0; 3]
            }
        })
        .collect()
}

fn build_grid(mesh: &BakeMesh, voxel: f32, bvh: &Bvh, with_albedo: bool) -> MeshSdf {
    let tris = &mesh.triangles;
    let bounds = mesh.bounds();
    let extent: [f32; 3] = std::array::from_fn(|a| (bounds.max[a] - bounds.min[a]).max(0.0));
    let albedo_of = if with_albedo {
        material_albedo(mesh)
    } else {
        Vec::new()
    };
    let band = BAND_VOXELS * voxel;
    // The grid reaches past the band, so every in-band point has a brick.
    let pad = band + voxel;
    let origin: [f32; 3] = std::array::from_fn(|a| bounds.min[a] - pad);
    let bricks_dims: [u32; 3] = std::array::from_fn(|a| {
        (((extent[a] + 2.0 * pad) / voxel / BRICK as f32).ceil() as u32).max(1)
    });
    let thin = !closed(tris);
    let brick_size = voxel * BRICK as f32;
    let coarse_band = BAND_VOXELS * brick_size;
    let signed = |p: [f32; 3], max: f32| -> f32 {
        let d = bvh.closest(tris, p, max).map_or(max, |(_, _, d)| d);
        if !thin && inside(bvh, tris, p) {
            -d
        } else {
            d
        }
    };
    let cells: Vec<[u32; 3]> = (0..bricks_dims[2])
        .flat_map(|z| {
            (0..bricks_dims[1]).flat_map(move |y| (0..bricks_dims[0]).map(move |x| [x, y, z]))
        })
        .collect();
    let centre = |c: [u32; 3]| -> [f32; 3] {
        std::array::from_fn(|a| origin[a] + (c[a] as f32 + 0.5) * brick_size)
    };
    let half_diag = brick_size * 0.5 * 3.0f32.sqrt();
    // Coarse grid and the list of bricks that touch the band, in parallel.
    let results: Vec<(u8, Option<(SdfBrick, [u8; 3])>)> = parallel_map(&cells, |&c| {
        let p = centre(c);
        let near = bvh.closest(tris, p, half_diag + band);
        let coarse = encode(signed(p, coarse_band), coarse_band);
        let brick = near.map(|(nearest, _, _)| {
            // Albedo: the nearest triangle's material at each voxel inside
            // the band, weighted towards the surface.
            let (mut colour, mut weight) = ([0.0f32; 3], 0.0f32);
            let distances = (0..BRICK * BRICK * BRICK)
                .map(|i| {
                    let l = [i % BRICK, (i / BRICK) % BRICK, i / (BRICK * BRICK)];
                    let q: [f32; 3] = std::array::from_fn(|a| {
                        origin[a] + ((c[a] as usize * BRICK + l[a]) as f32 + 0.5) * voxel
                    });
                    if with_albedo && l.iter().all(|v| v % 2 == 1) {
                        if let Some((tri, _, d)) = bvh.closest(tris, q, band) {
                            let m = tris[tri as usize].material as usize;
                            let w = (1.0 - d / band).max(0.0).powi(2) + 1.0e-4;
                            if let Some(a) = albedo_of.get(m) {
                                for k in 0..3 {
                                    colour[k] += a[k] * w;
                                }
                                weight += w;
                            }
                        }
                    }
                    encode(signed(q, band * 1.01), band)
                })
                .collect();
            let mean = if weight > 0.0 {
                colour.map(|v| ((v / weight).clamp(0.0, 1.0) * 255.0).round() as u8)
            } else {
                // No sampled voxel is inside the band (a brick off a sheet's
                // edge): the material nearest the brick centre.
                albedo_of
                    .get(tris[nearest as usize].material as usize)
                    .map_or([204; 3], |a| a.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            };
            (SdfBrick { at: c, distances }, mean)
        });
        (coarse, brick)
    });
    let mut coarse = Vec::with_capacity(results.len());
    let mut bricks = Vec::new();
    let mut brick_albedo = Vec::new();
    for (c, b) in results {
        coarse.push(c);
        if let Some((brick, mean)) = b {
            bricks.push(brick);
            if with_albedo {
                brick_albedo.push(mean);
            }
        }
    }
    MeshSdf {
        origin,
        voxel,
        bricks_dims,
        band,
        bricks,
        coarse,
        coarse_band,
        thin,
        brick_albedo,
    }
}

/// Every edge (by welded positions) belongs to exactly two triangles.
fn closed(tris: &[MeshTriangle]) -> bool {
    let key = |p: [f32; 3]| p.map(|x| (x * 1.0e4).round() as i64);
    let mut edges: HashMap<([i64; 3], [i64; 3]), u32> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (key(t.positions[k]), key(t.positions[(k + 1) % 3]));
            let e = if a < b { (a, b) } else { (b, a) };
            *edges.entry(e).or_default() += 1;
        }
    }
    !tris.is_empty() && edges.values().all(|&n| n == 2)
}

/// Majority of three parity tests along skew directions (no axis-aligned ray can
/// graze an edge of an axis-aligned box).
fn inside(bvh: &Bvh, tris: &[MeshTriangle], p: [f32; 3]) -> bool {
    const DIRS: [[f32; 3]; 3] = [
        [0.577_350_3, 0.577_350_3, 0.577_350_3],
        [-0.645_497_2, 0.516_397_8, -0.563_472_6],
        [0.267_261_2, -0.534_522_5, -0.801_783_7],
    ];
    DIRS.iter()
        .filter(|d| bvh.crossings(tris, p, **d) % 2 == 1)
        .count()
        >= 2
}

/// Map over `items` on every core, keeping the order.
pub(crate) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(threads * 8).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut parts: Vec<(usize, Vec<R>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let start = next.fetch_add(chunk, std::sync::atomic::Ordering::Relaxed);
                        if start >= items.len() {
                            break;
                        }
                        let end = (start + chunk).min(items.len());
                        out.push((start, items[start..end].iter().map(&f).collect()));
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
    parts.sort_by_key(|p| p.0);
    parts.into_iter().flat_map(|p| p.1).collect()
}
