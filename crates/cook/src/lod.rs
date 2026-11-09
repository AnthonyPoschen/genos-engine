//! Discrete LOD chain for one mesh (geometry LOD stage 1).
//!
//! Four levels, simplified from the imported triangles with `meshopt_simplify`
//! at 50%, 25%, and 12.5%. Level 0 is the import, error 0.
//! The renderer does not link this module's crate dependency: `genos-render`
//! does not depend on `genos-cook`.
//!
//! `meshopt_simplify` reports the max collapse residual (`meshopt_SimplifyErrorAbsolute`,
//! metres). That residual can sit below the distance between the two surfaces.
//! The stored error is a two-sided upper bound. Distance-to-surface is
//! 1-Lipschitz, so a sample at a sub-triangle's centroid plus that sample's
//! covering radius (the farthest vertex) bounds every point of the
//! sub-triangle. The cook splits until the radius is at most
//! [`LOD_COVER_STOP_M`], and only where `distance + radius` can raise the
//! running maximum. The simplifier residual is not that bound: it can sit
//! above or below the surface distance. A non-finite sample or residual is
//! not stored as zero.
//!
//! Each primitive is simplified with `LockBorder`, so a shared edge between
//! primitives stays on the imported vertices. An open n×n sheet then keeps its
//! 4n boundary vertices, and a triangulation of that loop has at least 4n−2
//! triangles.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use genos_load::{GltfMesh, MeshLod, Primitive, Sha256};
use genos_mesh::{Bvh, MeshTriangle};

/// Levels in a chain, including the imported mesh.
pub const LOD_COUNT: u32 = 4;
/// Triangle targets for levels 1, 2, and 3, as fractions of level 0.
pub const LOD_RATIOS: [f32; 3] = [0.5, 0.25, 0.125];
/// The same targets as eighths of the source count: 4/8, 2/8, 1/8.
/// The index target is this integer ratio, so the cook does not cast a rounded float.
const LOD_EIGHTHS: [usize; 3] = [4, 2, 1];
/// Stamped into the cook hash. Borders stay locked.
const LOD_LOCK_BORDER: u8 = 1;
/// Stamped into the cook hash. 2 = 1-Lipschitz cover, both directions.
const LOD_ERROR_BOUND: u8 = 2;
/// Stop splitting a sub-triangle once its covering radius is at most this, metres.
/// 1/256 is exact in `f32`. The stored bound is then the measured distance
/// plus at most this radius.
pub const LOD_COVER_STOP_M: f32 = 1.0 / 256.0;
/// Split cap. A metre-scale triangle reaches the stop long before this.
const COVER_DEPTH: u32 = 16;
/// Absolute error cap passed to `meshopt_simplify`, metres.
/// The target is the triangle count. This cap only stops a collapse that
/// would move the surface by more than this. A game mesh is far smaller.
pub const LOD_TARGET_ERROR_M: f32 = 1.0e6;

pub(crate) fn stamp_settings(hasher: &mut Sha256) {
    hasher.update(&LOD_COUNT.to_le_bytes());
    for ratio in LOD_RATIOS {
        hasher.update(&ratio.to_bits().to_le_bytes());
    }
    hasher.update(&LOD_TARGET_ERROR_M.to_bits().to_le_bytes());
    hasher.update(&[LOD_LOCK_BORDER, LOD_ERROR_BOUND]);
    hasher.update(&LOD_COVER_STOP_M.to_bits().to_le_bytes());
    hasher.update(&COVER_DEPTH.to_le_bytes());
}

/// The four-level chain. Level 0 is `mesh` unchanged.
#[must_use]
pub fn lod_chain(mesh: &GltfMesh) -> Vec<MeshLod> {
    let mut levels = Vec::with_capacity(usize::try_from(LOD_COUNT).unwrap_or(4));
    levels.push(MeshLod {
        error_m: 0.0,
        primitives: mesh.primitives.clone(),
    });
    for eighths in LOD_EIGHTHS {
        let prev_error = levels.last().map_or(0.0, |level| level.error_m);
        let mut error = prev_error;
        let mut primitives = Vec::with_capacity(mesh.primitives.len());
        for (index, prim) in mesh.primitives.iter().enumerate() {
            let (next, residual) = simplify_primitive(prim, eighths);
            let next_error = combined_error(residual, surface_bound(prim, &next));
            let prev_count = levels
                .last()
                .and_then(|level| level.primitives.get(index))
                .map_or(usize::MAX, |prev| prev.indices.len());
            if next.indices.len() <= prev_count {
                error = error.max(next_error);
                primitives.push(next);
            } else if let Some(prev) = levels.last().and_then(|level| level.primitives.get(index)) {
                primitives.push(prev.clone());
            } else {
                primitives.push(next);
            }
        }
        levels.push(MeshLod {
            error_m: error,
            primitives,
        });
    }
    levels
}

fn simplify_primitive(prim: &Primitive, eighths: usize) -> (Primitive, f32) {
    if !indexed(prim) {
        return (prim.clone(), 0.0);
    }
    let target = target_indices(prim.indices.len(), eighths);
    if target >= prim.indices.len() {
        return (prim.clone(), 0.0);
    }
    let mut error = 0.0_f32;
    let simplified = meshopt::simplify_decoder(
        &prim.indices,
        &prim.positions,
        target,
        LOD_TARGET_ERROR_M,
        meshopt::SimplifyOptions::ErrorAbsolute | meshopt::SimplifyOptions::LockBorder,
        Some(&mut error),
    );
    if simplified.len() < 3
        || !simplified.len().is_multiple_of(3)
        || simplified.len() > prim.indices.len()
    {
        return (prim.clone(), 0.0);
    }
    let residual = if !error.is_finite() {
        f32::INFINITY
    } else if error > 0.0 {
        error
    } else {
        0.0
    };
    compact(prim, &simplified).map_or((prim.clone(), f32::INFINITY), |next| (next, residual))
}

/// Object-space metres. The stored value is the Lipschitz bound. The residual
/// is checked only so a non-finite simplifier report cannot become a finite
/// error. `None` is a non-finite bound and must not be stored as zero.
fn combined_error(residual: f32, deviation: Option<f32>) -> f32 {
    match deviation {
        Some(dev) if dev.is_finite() && residual.is_finite() => dev,
        _ => f32::INFINITY,
    }
}

/// Upper bound on the distance between the two surfaces, both directions.
/// `None` when a sample, a radius, or a closest-point distance is non-finite.
fn surface_bound(source: &Primitive, level: &Primitive) -> Option<f32> {
    if same_triangle_set(source, level)? {
        return Some(0.0);
    }
    let fine_to_coarse = directed_bound(source, level)?;
    let coarse_to_fine = directed_bound(level, source)?;
    let worst = fine_to_coarse.max(coarse_to_fine);
    worst.is_finite().then_some(worst)
}

/// `Some(true)` when both primitives are the same set of triangles, so the
/// Hausdorff distance is 0. `None` when a corner is non-finite.
fn same_triangle_set(source: &Primitive, level: &Primitive) -> Option<bool> {
    let mut left = triangle_keys(source)?;
    let mut right = triangle_keys(level)?;
    left.sort_unstable();
    right.sort_unstable();
    Some(left == right)
}

fn triangle_keys(prim: &Primitive) -> Option<Vec<[[u32; 3]; 3]>> {
    let mut keys = Vec::new();
    for tri in prim.indices.as_chunks::<3>().0 {
        let Some(corners) = triangle_corners(prim, tri) else {
            continue;
        };
        if corners.iter().flatten().any(|axis| !axis.is_finite()) {
            return None;
        }
        let mut key = corners.map(|point| point.map(f32::to_bits));
        key.sort_unstable();
        keys.push(key);
    }
    Some(keys)
}

/// One sub-triangle waiting to be split or accepted.
/// `upper` is `distance(centroid) + covering radius`, an upper bound on the
/// distance from any point of the sub-triangle to `onto`.
#[derive(Clone, Copy)]
struct Cover {
    corners: [[f32; 3]; 3],
    upper: f32,
    radius: f32,
    depth: u32,
    id: u64,
}

impl PartialEq for Cover {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Cover {}

impl PartialOrd for Cover {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Cover {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.upper.total_cmp(&other.upper) {
            Ordering::Equal => other.id.cmp(&self.id),
            Ordering::Less => Ordering::Less,
            Ordering::Greater => Ordering::Greater,
        }
    }
}

fn directed_bound(from: &Primitive, onto: &Primitive) -> Option<f32> {
    let onto_tris = primitive_triangles(onto);
    let mut from_tris = Vec::new();
    for tri in from.indices.as_chunks::<3>().0 {
        if let Some(corners) = triangle_corners(from, tri) {
            if corners.iter().flatten().any(|axis| !axis.is_finite()) {
                return None;
            }
            from_tris.push(corners);
        }
    }
    if from_tris.is_empty() {
        return Some(0.0);
    }
    if onto_tris.is_empty() {
        return None;
    }
    let bvh = Bvh::build(&onto_tris);
    let reach = diagonal(&from.positions)
        .max(diagonal(&onto.positions))
        .max(1.0)
        * 2.0;
    if !reach.is_finite() {
        return None;
    }
    let mut next_id = 0_u64;
    let mut heap = BinaryHeap::new();
    for corners in from_tris {
        heap.push(cover_of(&bvh, &onto_tris, corners, 0, reach, &mut next_id)?);
    }
    while let Some(region) = heap.pop() {
        if region.radius <= LOD_COVER_STOP_M || region.depth >= COVER_DEPTH {
            return region.upper.is_finite().then_some(region.upper);
        }
        for child in subdivide(region.corners) {
            heap.push(cover_of(
                &bvh,
                &onto_tris,
                child,
                region.depth.saturating_add(1),
                reach,
                &mut next_id,
            )?);
        }
    }
    Some(0.0)
}

fn cover_of(
    bvh: &Bvh,
    tris: &[MeshTriangle],
    corners: [[f32; 3]; 3],
    depth: u32,
    reach: f32,
    next_id: &mut u64,
) -> Option<Cover> {
    let sample = centroid(corners);
    if sample.iter().any(|axis| !axis.is_finite()) {
        return None;
    }
    let radius = covering_radius(sample, corners)?;
    let distance = match bvh.closest(tris, sample, reach) {
        Some((_, _, distance)) if distance.is_finite() => padded_distance(distance, sample),
        _ => return None,
    };
    let upper = f32::next_up(distance + radius);
    if !upper.is_finite() {
        return None;
    }
    let id = *next_id;
    *next_id = next_id.saturating_add(1);
    Some(Cover {
        corners,
        upper,
        radius,
        depth,
        id,
    })
}

/// Farthest vertex from `sample`. Distance to a fixed point is convex, so the
/// max over a triangle is attained at a vertex. A few ulps of the extent cover
/// midpoint cracks between the four children.
fn covering_radius(sample: [f32; 3], corners: [[f32; 3]; 3]) -> Option<f32> {
    let mut radius = 0.0_f32;
    let mut extent = 0.0_f32;
    for corner in corners {
        let span = distance(sample, corner);
        if !span.is_finite() {
            return None;
        }
        radius = radius.max(span);
        for axis in corner {
            extent = extent.max(axis.abs());
        }
    }
    let padded = radius + extent * (16.0 * f32::EPSILON);
    if !padded.is_finite() {
        return None;
    }
    Some(f32::next_up(padded))
}

fn padded_distance(distance: f32, sample: [f32; 3]) -> f32 {
    let extent = sample
        .into_iter()
        .fold(1.0_f32, |extent, axis| extent.max(axis.abs()));
    let padded = distance.max(0.0) + extent * (16.0 * f32::EPSILON);
    if padded.is_finite() {
        f32::next_up(padded)
    } else {
        padded
    }
}

fn subdivide(corners: [[f32; 3]; 3]) -> [[[f32; 3]; 3]; 4] {
    let [a, b, c] = corners;
    let ab = midpoint(a, b);
    let bc = midpoint(b, c);
    let ca = midpoint(c, a);
    [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]
}

fn centroid(corners: [[f32; 3]; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| {
        (corners[0][axis] + corners[1][axis] + corners[2][axis]) * (1.0 / 3.0)
    })
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let span = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt()
}

fn triangle_corners(prim: &Primitive, tri: &[u32; 3]) -> Option<[[f32; 3]; 3]> {
    let corner = |slot: usize| {
        tri.get(slot)
            .and_then(|index| usize::try_from(*index).ok())
            .and_then(|index| prim.positions.get(index).copied())
    };
    Some([corner(0)?, corner(1)?, corner(2)?])
}

fn midpoint(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| f32::midpoint(a[axis], b[axis]))
}

fn primitive_triangles(level: &Primitive) -> Vec<MeshTriangle> {
    level
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|tri| Some(MeshTriangle::flat(triangle_corners(level, tri)?, 0, true)))
        .collect()
}

fn diagonal(points: &[[f32; 3]]) -> f32 {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in points {
        for axis in 0..3 {
            if let (Some(lo), Some(hi), Some(value)) =
                (min.get_mut(axis), max.get_mut(axis), point.get(axis))
            {
                *lo = lo.min(*value);
                *hi = hi.max(*value);
            }
        }
    }
    let span = [0, 1, 2]
        .map(|axis| max.get(axis).copied().unwrap_or(0.0) - min.get(axis).copied().unwrap_or(0.0));
    (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt()
}

fn indexed(prim: &Primitive) -> bool {
    prim.indices.len() >= 6
        && prim.indices.len().is_multiple_of(3)
        && !prim.positions.is_empty()
        && prim
            .indices
            .iter()
            .all(|index| usize::try_from(*index).is_ok_and(|i| prim.positions.get(i).is_some()))
}

fn target_indices(index_count: usize, eighths: usize) -> usize {
    let tris = index_count / 3;
    if tris == 0 || eighths == 0 {
        return 0;
    }
    // Nearest count at `eighths/8`, half rounding away from zero.
    let want = tris.saturating_mul(eighths).saturating_add(4) / 8;
    let want = want.clamp(1, tris);
    want.saturating_mul(3).min(index_count)
}

fn compact(src: &Primitive, indices: &[u32]) -> Option<Primitive> {
    let verts = src.positions.len();
    let copy_normals = present(&src.normals, verts)?;
    let copy_uv = present(&src.texcoords, verts)?;
    let copy_tangents = present(&src.tangents, verts)?;
    for set in &src.texcoord_sets {
        present(set, verts)?;
    }
    let mut remap = vec![u32::MAX; verts];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut texcoords = Vec::new();
    let mut tangents = Vec::new();
    let mut texcoord_sets: Vec<Vec<[f32; 2]>> = src
        .texcoord_sets
        .iter()
        .map(|set| Vec::with_capacity(if set.is_empty() { 0 } else { indices.len() }))
        .collect();
    let mut out_indices = Vec::with_capacity(indices.len());
    for index in indices {
        let source = usize::try_from(*index).ok()?;
        let slot = if let Some(slot) = remap.get(source).copied().filter(|slot| *slot != u32::MAX) {
            slot
        } else {
            let slot = u32::try_from(positions.len()).ok()?;
            positions.push(*src.positions.get(source)?);
            if copy_normals {
                normals.push(*src.normals.get(source)?);
            }
            if copy_uv {
                texcoords.push(*src.texcoords.get(source)?);
            }
            if copy_tangents {
                tangents.push(*src.tangents.get(source)?);
            }
            for (out, set) in texcoord_sets.iter_mut().zip(&src.texcoord_sets) {
                if !set.is_empty() {
                    out.push(*set.get(source)?);
                }
            }
            *remap.get_mut(source)? = slot;
            slot
        };
        out_indices.push(slot);
    }
    Some(Primitive {
        positions,
        normals,
        texcoords,
        texcoord_sets,
        tangents,
        indices: out_indices,
        material: src.material,
    })
}

/// Empty is absent. The same length as the positions is present. Anything else
/// is a broken primitive and is not simplified.
fn present<T>(src: &[T], verts: usize) -> Option<bool> {
    if src.is_empty() {
        Some(false)
    } else if src.len() == verts {
        Some(true)
    } else {
        None
    }
}

/// The chain a record stores. An empty chain is the imported mesh at error 0,
/// so a hand-built record still encodes.
pub(crate) fn levels_of(mesh: &crate::CookedMesh) -> Vec<MeshLod> {
    if mesh.lods.is_empty() {
        vec![MeshLod {
            error_m: 0.0,
            primitives: mesh.mesh.primitives.clone(),
        }]
    } else {
        mesh.lods.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{combined_error, surface_bound, LOD_COVER_STOP_M, LOD_EIGHTHS, LOD_RATIOS};
    use genos_load::Primitive;

    #[test]
    fn eighths_match_the_published_ratios() {
        assert_eq!(LOD_RATIOS.len(), LOD_EIGHTHS.len());
        for (&ratio, eighths) in LOD_RATIOS.iter().zip(LOD_EIGHTHS) {
            let scaled = u8::try_from(eighths).unwrap_or(u8::MAX);
            assert_eq!((ratio * 8.0).to_bits(), f32::from(scaled).to_bits());
        }
    }

    #[test]
    fn a_non_finite_deviation_is_not_stored_as_zero() {
        let mut source = Primitive {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..Primitive::default()
        };
        let level = source.clone();
        source.positions[0][2] = f32::NAN;
        assert!(surface_bound(&source, &level).is_none());
        let stored = combined_error(0.0, None);
        assert!(!stored.is_finite());
        assert_ne!(stored.to_bits(), 0.0_f32.to_bits());
        assert!(!combined_error(f32::NAN, Some(0.25)).is_finite());
    }

    fn flat_triangle(z: f32) -> Primitive {
        Primitive {
            positions: vec![[0.0, 0.0, z], [1.0, 0.0, z], [0.0, 1.0, z]],
            indices: vec![0, 1, 2],
            ..Primitive::default()
        }
    }

    #[test]
    fn the_cover_bounds_a_rigid_lift() {
        let bound = surface_bound(&flat_triangle(0.0), &flat_triangle(0.2)).unwrap_or(f32::NAN);
        assert!(
            bound >= 0.2,
            "lift of 0.2 m stored {bound}, which is under the displacement"
        );
        assert!(
            bound <= 0.2 + LOD_COVER_STOP_M,
            "lift of 0.2 m stored {bound}, looser than the stop radius"
        );
    }
}
