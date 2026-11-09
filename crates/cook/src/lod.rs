//! Discrete LOD chain for one mesh (geometry LOD stage 1).
//!
//! Four levels, simplified from the imported triangles with `meshopt_simplify`
//! at 50%, 25%, and 12.5%. Level 0 is the import, error 0.
//! The renderer does not link this module's crate dependency: `genos-render`
//! does not depend on `genos-cook`.
//!
//! `meshopt_simplify` reports the max collapse residual (`meshopt_SimplifyErrorAbsolute`,
//! metres). That residual can sit below the distance from a level-0 vertex to
//! the simplified surface, because later collapses move the surface away from
//! an earlier residual. The stored error is the max of the residual and that
//! vertex distance (the mesh BVH closest-point query).

use genos_load::{GltfMesh, MeshLod, Primitive, Sha256};
use genos_mesh::{Bvh, MeshTriangle};

/// Levels in a chain, including the imported mesh.
pub const LOD_COUNT: u32 = 4;
/// Triangle targets for levels 1, 2, and 3, as fractions of level 0.
pub const LOD_RATIOS: [f32; 3] = [0.5, 0.25, 0.125];
/// The same targets as eighths of the source count: 4/8, 2/8, 1/8.
/// The index target is this integer ratio, so the cook does not cast a rounded float.
const LOD_EIGHTHS: [usize; 3] = [4, 2, 1];
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
            let next_error = residual.max(vertex_deviation(prim, &next));
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
        meshopt::SimplifyOptions::ErrorAbsolute,
        Some(&mut error),
    );
    if simplified.len() < 3
        || !simplified.len().is_multiple_of(3)
        || simplified.len() > prim.indices.len()
    {
        return (prim.clone(), 0.0);
    }
    let error = if error.is_finite() && error > 0.0 {
        error
    } else {
        0.0
    };
    compact(prim, &simplified).map_or((prim.clone(), 0.0), |next| (next, error))
}

/// Max distance from a level-0 vertex to the simplified surface.
fn vertex_deviation(source: &Primitive, level: &Primitive) -> f32 {
    let tris = coarse_triangles(level);
    if tris.is_empty() {
        return if source.positions.is_empty() {
            0.0
        } else {
            f32::MAX
        };
    }
    let bvh = Bvh::build(&tris);
    let reach = diagonal(&source.positions).max(1.0);
    let mut worst = 0.0_f32;
    for point in &source.positions {
        worst = match bvh.closest(&tris, *point, reach) {
            Some((_, _, distance)) => worst.max(distance),
            None => worst.max(reach),
        };
    }
    if worst.is_finite() {
        worst
    } else {
        0.0
    }
}

fn coarse_triangles(level: &Primitive) -> Vec<MeshTriangle> {
    level
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|tri| {
            let corner = |slot: usize| {
                tri.get(slot)
                    .and_then(|index| usize::try_from(*index).ok())
                    .and_then(|index| level.positions.get(index).copied())
            };
            Some(MeshTriangle::flat(
                [corner(0)?, corner(1)?, corner(2)?],
                0,
                true,
            ))
        })
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
    use super::{LOD_EIGHTHS, LOD_RATIOS};

    #[test]
    fn eighths_match_the_published_ratios() {
        assert_eq!(LOD_RATIOS.len(), LOD_EIGHTHS.len());
        for (&ratio, eighths) in LOD_RATIOS.iter().zip(LOD_EIGHTHS) {
            let scaled = u8::try_from(eighths).unwrap_or(u8::MAX);
            assert_eq!((ratio * 8.0).to_bits(), f32::from(scaled).to_bits());
        }
    }
}
