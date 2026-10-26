//! Offline Voronoi fracture and the `.gfract` version-1 asset.
//!
//! The step does not cook. It reads the asset and swaps chunks. Seed order is
//! cell order. Planes are applied in increasing seed index.

mod sha256;
mod voronoi;

use genos_math::Vec3;

pub use sha256::digest;
pub use voronoi::{box_cell, volume, voronoi, Cell, HULL};

/// A cook or a file that cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FractureError {
    ThinCell { seed: u32 },
    BadVersion,
    ShortFile,
    BadCount,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub parent: u32,
    pub first_child: u32,
    pub child_count: u32,
    pub volume: f32,
    pub support: bool,
    pub center: Vec3,
    pub half: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bond {
    pub chunk_a: u32,
    pub chunk_b: u32,
    pub area: f32,
    pub health: f32,
    pub health_rest: f32,
    pub centroid: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub health_per_area: f32,
    pub chunks: Vec<Chunk>,
    pub bonds: Vec<Bond>,
}

#[derive(Clone, Debug)]
pub struct CookRequest {
    pub min: Vec3,
    pub max: Vec3,
    pub coarse: Vec<Vec3>,
    pub support: Vec<bool>,
    pub children_per_chunk: u8,
    pub health_per_area: f32,
}

/// Clip the box into coarse cells, then a fixed grid of children inside each cell.
///
/// # Errors
///
/// Returns [`FractureError::ThinCell`] when a seed collapses a cell.
pub fn cook(request: &CookRequest) -> Result<Asset, FractureError> {
    let hull = box_cell(request.min, request.max);
    let coarse_cells = voronoi(&hull, &request.coarse)?;
    let mut chunks = Vec::new();
    let mut cells = Vec::new();
    for (index, cell) in coarse_cells.into_iter().enumerate() {
        let support = request.support.get(index).copied().unwrap_or(false);
        chunks.push(chunk_from_cell(&cell, HULL, 0, 0, support));
        cells.push(cell);
    }
    let coarse_count = chunks.len();
    if request.children_per_chunk > 0 {
        for index in 0..coarse_count {
            let parent = u32::try_from(index).unwrap_or(u32::MAX);
            let seeds = child_seeds(&cells[index], request.children_per_chunk);
            let children = voronoi(&cells[index], &seeds)?;
            let first = u32::try_from(chunks.len()).unwrap_or(u32::MAX);
            let count = u32::try_from(children.len()).unwrap_or(0);
            chunks[index].first_child = first;
            chunks[index].child_count = count;
            for child in children {
                chunks.push(chunk_from_cell(&child, parent, 0, 0, false));
            }
        }
    }
    let mut bonds = bonds_between(&cells, 0, request.health_per_area);
    for index in 0..coarse_count {
        let first = usize::try_from(chunks[index].first_child).unwrap_or(0);
        let count = usize::try_from(chunks[index].child_count).unwrap_or(0);
        if count == 0 {
            continue;
        }
        let child_cells = voronoi(
            &cells[index],
            &child_seeds(&cells[index], request.children_per_chunk),
        )?;
        let mut child_bonds = bonds_between(&child_cells, first, request.health_per_area);
        bonds.append(&mut child_bonds);
    }
    for (index, chunk) in chunks.iter().enumerate().take(coarse_count) {
        if chunk.support {
            bonds.push(world_bond(
                u32::try_from(index).unwrap_or(0),
                request.health_per_area,
            ));
        }
    }
    Ok(Asset {
        health_per_area: request.health_per_area,
        chunks,
        bonds,
    })
}

fn chunk_from_cell(
    cell: &Cell,
    parent: u32,
    first_child: u32,
    child_count: u32,
    support: bool,
) -> Chunk {
    let (min, max) = voronoi::bounds(cell);
    let half = (max - min) * 0.5;
    Chunk {
        parent,
        first_child,
        child_count,
        volume: volume(cell),
        support,
        center: (min + max) * 0.5,
        half,
    }
}

fn child_seeds(cell: &Cell, count: u8) -> Vec<Vec3> {
    let (min, max) = voronoi::bounds(cell);
    let center = voronoi::centroid(cell);
    let extent = max - min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let mut seeds = Vec::new();
    let total = u16::from(count).max(1);
    for index in 0..total {
        let t = (f32::from(index) + 0.5) / f32::from(total);
        let along = axis_get(min, axis) + axis_get(extent, axis) * (0.2 + 0.6 * t);
        seeds.push(axis_set(center, axis, along));
    }
    seeds
}

fn axis_get(value: Vec3, axis: usize) -> f32 {
    match axis {
        0 => value.x,
        1 => value.y,
        _ => value.z,
    }
}

fn axis_set(mut value: Vec3, axis: usize, along: f32) -> Vec3 {
    match axis {
        0 => value.x = along,
        1 => value.y = along,
        _ => value.z = along,
    }
    value
}

fn bonds_between(cells: &[Cell], index_base: usize, health_per_area: f32) -> Vec<Bond> {
    let mut bonds = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        for (face_index, against) in cell.against.iter().enumerate() {
            let other = usize::try_from(*against).unwrap_or(usize::MAX);
            if *against == HULL || other <= index {
                continue;
            }
            let Some(face) = cell.faces.get(face_index) else {
                continue;
            };
            let area = voronoi::face_area(cell, face);
            if area <= 1.0e-8 {
                continue;
            }
            let health = health_per_area * area;
            bonds.push(Bond {
                chunk_a: u32::try_from(index_base + index).unwrap_or(0),
                chunk_b: u32::try_from(index_base + other).unwrap_or(0),
                area,
                health,
                health_rest: health,
                centroid: voronoi::face_centroid(cell, face),
                normal: voronoi::face_normal_of(cell, face),
            });
        }
    }
    bonds
}

fn world_bond(chunk: u32, health_per_area: f32) -> Bond {
    let health = health_per_area * 10.0;
    Bond {
        chunk_a: chunk,
        chunk_b: HULL,
        area: 0.0,
        health,
        health_rest: health,
        centroid: Vec3::ZERO,
        normal: Vec3::Y,
    }
}

const MAGIC: &[u8; 4] = b"GFR1";

/// Version 1, little-endian. The SHA-256 of these bytes is the sim content hash.
#[must_use]
pub fn write_gfract(asset: &Asset) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, u32::try_from(asset.chunks.len()).unwrap_or(0));
    push_u32(&mut bytes, u32::try_from(asset.bonds.len()).unwrap_or(0));
    push_f32(&mut bytes, asset.health_per_area);
    for chunk in &asset.chunks {
        push_u32(&mut bytes, chunk.parent);
        push_u32(&mut bytes, chunk.first_child);
        push_u32(&mut bytes, chunk.child_count);
        push_f32(&mut bytes, chunk.volume);
        push_u32(&mut bytes, u32::from(chunk.support));
        push_vec(&mut bytes, chunk.center);
        push_vec(&mut bytes, chunk.half);
    }
    for bond in &asset.bonds {
        push_u32(&mut bytes, bond.chunk_a);
        push_u32(&mut bytes, bond.chunk_b);
        push_f32(&mut bytes, bond.area);
        push_f32(&mut bytes, bond.health);
        push_f32(&mut bytes, bond.health_rest);
        push_vec(&mut bytes, bond.centroid);
        push_vec(&mut bytes, bond.normal);
    }
    bytes
}

/// # Errors
///
/// Returns [`FractureError::BadVersion`] for a foreign magic or version, and
/// [`FractureError::ShortFile`] when the buffer ends early.
pub fn read_gfract(bytes: &[u8]) -> Result<Asset, FractureError> {
    let mut reader = Reader { bytes, at: 0 };
    let magic = reader.bytes_n(4)?;
    if magic != MAGIC {
        return Err(FractureError::BadVersion);
    }
    let version = reader.u32()?;
    if version != 1 {
        return Err(FractureError::BadVersion);
    }
    let chunk_count = reader.u32()?;
    let bond_count = reader.u32()?;
    if chunk_count > 10_000 || bond_count > 10_000 {
        return Err(FractureError::BadCount);
    }
    let health_per_area = reader.f32()?;
    let mut chunks = Vec::new();
    for _ in 0..chunk_count {
        chunks.push(Chunk {
            parent: reader.u32()?,
            first_child: reader.u32()?,
            child_count: reader.u32()?,
            volume: reader.f32()?,
            support: reader.u32()? != 0,
            center: reader.vec()?,
            half: reader.vec()?,
        });
    }
    let mut bonds = Vec::new();
    for _ in 0..bond_count {
        bonds.push(Bond {
            chunk_a: reader.u32()?,
            chunk_b: reader.u32()?,
            area: reader.f32()?,
            health: reader.f32()?,
            health_rest: reader.f32()?,
            centroid: reader.vec()?,
            normal: reader.vec()?,
        });
    }
    Ok(Asset {
        health_per_area,
        chunks,
        bonds,
    })
}

#[must_use]
pub fn content_hash(bytes: &[u8]) -> [u8; 32] {
    digest(bytes)
}

impl Asset {
    /// Bond connecting `a` and `b` in either order.
    #[must_use]
    pub fn bond_between(&self, a: u32, b: u32) -> Option<usize> {
        self.bonds.iter().position(|bond| {
            (bond.chunk_a == a && bond.chunk_b == b) || (bond.chunk_a == b && bond.chunk_b == a)
        })
    }
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_vec(bytes: &mut Vec<u8>, value: Vec3) {
    push_f32(bytes, value.x);
    push_f32(bytes, value.y);
    push_f32(bytes, value.z);
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes_n(&mut self, count: usize) -> Result<&[u8], FractureError> {
        let end = self.at.checked_add(count).ok_or(FractureError::ShortFile)?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(FractureError::ShortFile)?;
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, FractureError> {
        let mut buf = [0_u8; 4];
        buf.copy_from_slice(self.bytes_n(4)?);
        Ok(u32::from_le_bytes(buf))
    }

    fn f32(&mut self) -> Result<f32, FractureError> {
        Ok(f32::from_le_bytes(self.u32()?.to_le_bytes()))
    }

    fn vec(&mut self) -> Result<Vec3, FractureError> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
}

#[cfg(test)]
mod tests {
    use genos_math::Vec3;

    use super::{
        box_cell, content_hash, cook, digest, read_gfract, volume, voronoi, write_gfract,
        CookRequest, FractureError, HULL,
    };

    #[test]
    fn sha256_of_empty_matches_the_known_digest() {
        let hash = digest(b"");
        let mut text = String::new();
        for byte in hash {
            let pair = format!("{byte:02x}");
            text.push_str(&pair);
        }
        assert_eq!(
            text,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn two_seeds_split_a_cube_into_equal_volumes() {
        let hull = box_cell(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let seeds = [Vec3::new(-0.5, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0)];
        let cells = voronoi(&hull, &seeds).unwrap_or_default();
        assert_eq!(cells.len(), 2);
        let left = volume(&cells[0]);
        let right = volume(&cells[1]);
        assert!((left - 4.0).abs() < 1.0e-3, "left volume {left}");
        assert!((right - 4.0).abs() < 1.0e-3, "right volume {right}");
        let again = voronoi(&hull, &seeds).unwrap_or_default();
        assert_eq!(cells[0].vertices, again[0].vertices);
        assert_eq!(cells[1].vertices, again[1].vertices);
    }

    #[test]
    fn coarse_chunks_round_trip_and_contain_their_children() {
        let asset = cook(&CookRequest {
            min: Vec3::new(-1.0, -1.0, -1.0),
            max: Vec3::new(1.0, 1.0, 1.0),
            coarse: vec![Vec3::new(-0.5, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0)],
            support: vec![true, false],
            children_per_chunk: 2,
            health_per_area: 10.0,
        })
        .unwrap_or_else(|_| empty_asset());
        assert_eq!(asset.chunks.len(), 6, "2 coarse + 4 fine");
        let world_bonds = asset
            .bonds
            .iter()
            .filter(|bond| bond.chunk_b == HULL)
            .count();
        assert_eq!(world_bonds, 1);
        for chunk in asset.chunks.iter().skip(2) {
            let parent = usize::try_from(chunk.parent).unwrap_or(0);
            let parent_chunk = &asset.chunks[parent];
            let delta = chunk.center - parent_chunk.center;
            assert!(delta.x.abs() <= parent_chunk.half.x + 1.0e-3);
            assert!(delta.y.abs() <= parent_chunk.half.y + 1.0e-3);
            assert!(delta.z.abs() <= parent_chunk.half.z + 1.0e-3);
        }
        let bytes = write_gfract(&asset);
        let loaded = read_gfract(&bytes).unwrap_or_else(|_| empty_asset());
        assert_eq!(loaded, asset);
        assert_eq!(content_hash(&bytes), digest(&bytes));
        assert!(matches!(
            read_gfract(&[0, 0, 0, 0, 2, 0, 0, 0]),
            Err(FractureError::BadVersion)
        ));
    }

    fn empty_asset() -> super::Asset {
        super::Asset {
            health_per_area: 0.0,
            chunks: Vec::new(),
            bonds: Vec::new(),
        }
    }
}
