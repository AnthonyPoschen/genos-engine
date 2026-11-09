//! Discrete LOD chain for one mesh (geometry LOD stage 1).
//!
//! Four levels, simplified from the imported triangles with `meshopt_simplify`
//! at 50%, 25%, and 12.5%. Level 0 is the import, error 0.
//! The renderer does not link this module's crate dependency: `genos-render`
//! does not depend on `genos-cook`.
//!
//! `meshopt_simplify` reports the max collapse residual (`meshopt_SimplifyErrorAbsolute`,
//! metres). That residual can sit below the distance between the two surfaces.
//! The stored error is the max of the residual and a two-sided sample of both
//! surfaces: vertices, edge midpoints, and face interiors, fine-to-coarse and
//! coarse-to-fine. A non-finite sample is not stored as zero.
//!
//! Each primitive is simplified with `LockBorder`, so a shared edge between
//! primitives stays on the imported vertices.

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
/// Stamped into the cook hash. The bound samples both surfaces.
const LOD_TWO_SIDED: u8 = 1;
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
    hasher.update(&[LOD_LOCK_BORDER, LOD_TWO_SIDED]);
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
            let next_error = combined_error(residual, surface_deviation(prim, &next));
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

/// Object-space metres. `None` is a non-finite sample: the caller must not
/// store that as a zero bound.
fn combined_error(residual: f32, deviation: Option<f32>) -> f32 {
    match deviation {
        Some(dev) if dev.is_finite() && residual.is_finite() => {
            residual.max(0.0).max(above_sample(dev))
        }
        _ => f32::INFINITY,
    }
}

/// A second closest-point implementation can land a few ulps above the BVH
/// sample. The stored bound sits above that residual. This is not a fraction
/// of the error.
fn above_sample(distance: f32) -> f32 {
    let mut value = distance;
    for _ in 0..32 {
        let up = f32::next_up(value);
        if !up.is_finite() {
            return distance;
        }
        value = up;
    }
    value
}

/// Max sampled distance between the two surfaces, both directions.
/// Vertices, edge midpoints, and face interiors. `None` when a sample or a
/// closest-point distance is non-finite, or the query finds nothing.
fn surface_deviation(source: &Primitive, level: &Primitive) -> Option<f32> {
    let fine_to_coarse = directed_deviation(source, level)?;
    let coarse_to_fine = directed_deviation(level, source)?;
    let worst = fine_to_coarse.max(coarse_to_fine);
    if worst.is_finite() {
        Some(worst)
    } else {
        None
    }
}

fn directed_deviation(from: &Primitive, onto: &Primitive) -> Option<f32> {
    let samples = surface_samples(from);
    if samples.is_empty() {
        return Some(0.0);
    }
    if samples
        .iter()
        .any(|point| point.iter().any(|axis| !axis.is_finite()))
    {
        return None;
    }
    let tris = primitive_triangles(onto);
    if tris.is_empty() {
        return None;
    }
    let bvh = Bvh::build(&tris);
    let reach = diagonal(&from.positions)
        .max(diagonal(&onto.positions))
        .max(1.0)
        * 2.0;
    if !reach.is_finite() {
        return None;
    }
    let mut worst = 0.0_f32;
    for point in samples {
        match bvh.closest(&tris, point, reach) {
            Some((_, _, distance)) if distance.is_finite() => worst = worst.max(distance),
            _ => return None,
        }
    }
    Some(worst)
}

/// Barycentric weights (a, b, c) for face-interior samples.
const FACE_INTERIORS: [[f32; 3]; 4] = [
    [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    [0.5, 0.25, 0.25],
    [0.25, 0.5, 0.25],
    [0.25, 0.25, 0.5],
];

fn surface_samples(prim: &Primitive) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for tri in prim.indices.as_chunks::<3>().0 {
        let Some(corners) = triangle_corners(prim, tri) else {
            continue;
        };
        let [a, b, c] = corners;
        out.extend([a, b, c, midpoint(a, b), midpoint(b, c), midpoint(c, a)]);
        for weights in FACE_INTERIORS {
            out.push(barycentric(a, b, c, weights));
        }
    }
    out
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

fn barycentric(a: [f32; 3], b: [f32; 3], c: [f32; 3], weights: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] * weights[0] + b[axis] * weights[1] + c[axis] * weights[2])
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
    use super::{combined_error, surface_deviation, LOD_EIGHTHS, LOD_RATIOS};
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
        assert!(surface_deviation(&source, &level).is_none());
        let stored = combined_error(0.0, None);
        assert!(!stored.is_finite());
        assert_ne!(stored.to_bits(), 0.0_f32.to_bits());
        assert!(!combined_error(f32::NAN, Some(0.25)).is_finite());
    }
}
