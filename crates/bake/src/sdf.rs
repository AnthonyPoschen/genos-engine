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

pub fn build(mesh: &BakeMesh, s: &BakeSettings) -> MeshSdf {
    let tris = &mesh.triangles;
    let bvh = Bvh::build(tris);
    let bounds = mesh.bounds();
    let extent: [f32; 3] = std::array::from_fn(|a| (bounds.max[a] - bounds.min[a]).max(0.0));
    let largest = extent.iter().fold(0.0f32, |m, &e| m.max(e));
    // A mesh far larger than a room (a whole level as one mesh) gets coarser voxels
    // instead of an unbounded grid: at most MAX_VOXELS_PER_SIDE along its largest side.
    let voxel = (largest / s.voxels_per_side)
        .clamp(s.voxel_min, s.voxel_max)
        .max(largest / MAX_VOXELS_PER_SIDE);
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
        if !thin && inside(&bvh, tris, p) {
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
    let results: Vec<(u8, Option<SdfBrick>)> = parallel_map(&cells, |&c| {
        let p = centre(c);
        let near = bvh.closest(tris, p, half_diag + band).is_some();
        let coarse = encode(signed(p, coarse_band), coarse_band);
        let brick = near.then(|| SdfBrick {
            at: c,
            distances: (0..BRICK * BRICK * BRICK)
                .map(|i| {
                    let l = [i % BRICK, (i / BRICK) % BRICK, i / (BRICK * BRICK)];
                    let q: [f32; 3] = std::array::from_fn(|a| {
                        origin[a] + ((c[a] as usize * BRICK + l[a]) as f32 + 0.5) * voxel
                    });
                    encode(signed(q, band * 1.01), band)
                })
                .collect(),
        });
        (coarse, brick)
    });
    let mut coarse = Vec::with_capacity(results.len());
    let mut bricks = Vec::new();
    for (c, b) in results {
        coarse.push(c);
        bricks.extend(b);
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
