//! The voxel field of the cone-traced cascades (experimental, off unless the
//! `cone_cascades` lighting setting is on; the plan is
//! `docs/systems/lighting/cone-traced-cascades-plan.md`, phase 4).
//!
//! The field is built on the CPU from the scene's surfaces when a scene loads.
//! Every surface point goes into the voxel behind it at each level from
//! 6.25 cm to 8 m. A voxel keeps, for each of the six axis ways, the area that
//! faces that way, the light it sends, the area's centroid and the box of its
//! points; and the lamps' glowing spheres apart. The GPU gets it as sparse
//! bricks: 8 x 8 x 8 voxels of one level, stored only where a surface or a
//! lamp is in the brick or one voxel from it, each with a one-voxel apron
//! copied from its neighbours so a filtered read is whole inside its brick.
//! One table per level gives each brick's place in the atlas.
//!
//! The same field, read the same way, is the CPU prototype's in
//! `debug/single-frame-cascade`.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::f32::consts::PI;

use genos_scene::Scene;

use crate::cache_probe::floor_i32;
use crate::{GltfSurface, SceneMesh};

type V3 = [f32; 3];

/// Voxel levels: a voxel of level `l` is `BASE * 2^l` metres.
pub const FIELD_FINEST: u8 = 2;
pub const FIELD_COARSEST: u8 = 9;
pub const FIELD_LEVELS: usize = 8;
const BASE: f32 = 1.0 / 64.0;
/// The levels whose planes the crossing test reads (6.25 and 12.5 cm).
pub const PLANE_COARSEST: u8 = 3;
/// Voxels along a brick's side, and with its apron.
pub const BRICK: i32 = 8;
pub const PADDED: i32 = BRICK + 2;
/// Bricks along the atlas's x and y; z grows with the count.
pub const ATLAS_BRICKS: u32 = 32;
/// A brick table entry with no brick.
pub const NO_BRICK: u32 = u32::MAX;
/// A lamp's sphere is shared over this many points.
const LAMP_POINTS: u32 = 256;
/// The engine's lamp unit: a lamp's colour times this is its power.
pub const LAMP_UNIT: f32 = 72.0;

const AXES: [V3; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

#[must_use]
pub fn voxel_size(level: u8) -> f32 {
    BASE * f32::from(1u16 << level)
}

#[derive(Clone, Copy)]
struct Voxel {
    area: [f32; 6],
    light: [V3; 6],
    at: [V3; 6],
    lo: [V3; 6],
    hi: [V3; 6],
    lamp_area: f32,
    lamp_light: V3,
}

impl Default for Voxel {
    fn default() -> Self {
        Self {
            area: [0.0; 6],
            light: [[0.0; 3]; 6],
            at: [[0.0; 3]; 6],
            lo: [[f32::INFINITY; 3]; 6],
            hi: [[f32::NEG_INFINITY; 3]; 6],
            lamp_area: 0.0,
            lamp_light: [0.0; 3],
        }
    }
}

/// One surface sample: point, the way it faces, its area, its albedo.
pub type FieldPoint = (V3, V3, f32, V3);

/// Where one level's bricks are: its table starts at `offset` in
/// [`ConeField::table`], covering `dims` bricks from brick `origin`. `clip`
/// is the level's clip volume, `[x, y, z, side]` in its voxels: the box from
/// that corner `side` voxels along each axis that the level holds. Past it
/// the level holds nothing and a coarser one stands in. A side of 0 holds the
/// whole scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FieldLevel {
    pub origin: [i32; 3],
    pub dims: [u32; 3],
    pub offset: u32,
    pub clip: [i32; 4],
}

/// Voxels across each clip level's volume around the eye (8 m at 6.25 cm,
/// 16 m at 12.5 cm, 32 m at 25 cm). The levels from [`WHOLE_FINEST`] up hold
/// the whole scene.
pub const CLIP_CELLS: i32 = 128;
/// The eye may move this far from the clip volumes' centre (metres) before
/// they are built again around it: a quarter of the finest volume's half.
pub const CLIP_MOVE: f32 = 2.0;
/// A clip volume that holds nothing: a fine level not built yet.
pub const NO_CLIP: [i32; 4] = [0, 0, 0, -1];
/// The first level kept for the whole scene (50 cm); finer ones are clip
/// volumes. A dense brick of 25 cm voxels over a whole Sponza was 140 MiB and
/// most of the first build.
pub const WHOLE_FINEST: u8 = PLANE_COARSEST + 2;

/// Words a fine brick's header holds: the index of its first voxel record,
/// 16 words of mask (bit `x + 8 y + 64 z` set where a voxel is), and for each
/// mask word the count of voxels in the words before it.
pub const FINE_HEADER: usize = 33;
/// Words a fine voxel record holds: for each way two words of light and area
/// (half2 r g, half2 b area), then for each way three words of its plane
/// (half2 centroid low-u, half2 high-u low-v, half2 high-v 0), in voxels from
/// the voxel's low corner, then two words of the lamp term, then the voxel's
/// seat word ([`seat_word`]).
pub const FINE_RECORD: usize = 33;

/// The field as the GPU takes it. The fine levels (6.25 and 12.5 cm), which
/// the march reads one voxel at a time, are compact: a header for each brick
/// and a record for each voxel. The coarse levels, which it reads filtered,
/// are dense bricks with an apron in an atlas.
pub struct ConeField {
    pub levels: [FieldLevel; FIELD_LEVELS],
    /// For every brick of every level: at a fine level the brick's index in
    /// [`ConeField::fine_bricks`], at a coarse level its slot in the atlas;
    /// [`NO_BRICK`] where there is none.
    pub table: Vec<u32>,
    pub fine_bricks: Vec<u32>,
    pub fine_voxels: Vec<u32>,
    pub coarse_bricks: u32,
    /// Texels along the atlas's x, y and z.
    pub atlas: [u32; 3],
    /// Seven RGBA16F atlases: the six ways (rgb light sent, a area), then the
    /// lamps (rgb light, a area).
    pub faces: [Vec<u16>; 7],
    /// One byte a texel: 255 where a surface is in the voxel or one of its 26
    /// neighbours.
    pub occupancy: Vec<u8>,
    /// One word a texel: the voxel's seat word ([`seat_word`]), 0 where none.
    pub coarse_seats: Vec<u32>,
    pub voxels: usize,
    pub points: usize,
}

impl ConeField {
    /// Bytes the field takes on the GPU.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.faces.iter().map(|f| f.len() * 2).sum::<usize>()
            + self.occupancy.len()
            + (self.table.len() + self.fine_bricks.len() + self.fine_voxels.len()) * 4
    }

    fn entry(&self, level: u8, cell: [i32; 3]) -> Option<u32> {
        let k = usize::from(level.checked_sub(FIELD_FINEST)?);
        let at = self.levels.get(k)?;
        let brick = cell.map(|c| c.div_euclid(BRICK));
        let mut rel = [0u32; 3];
        for a in 0..3 {
            rel[a] = u32::try_from(brick[a] - at.origin[a]).ok()?;
            if rel[a] >= at.dims[a] {
                return None;
            }
        }
        let index = at.offset + (rel[2] * at.dims[1] + rel[1]) * at.dims[0] + rel[0];
        let slot = *self.table.get(usize::try_from(index).ok()?)?;
        (slot != NO_BRICK).then_some(slot)
    }

    /// The texel of a coarse voxel in the atlases, or `None` where no brick is.
    #[must_use]
    pub fn texel(&self, level: u8, cell: [i32; 3]) -> Option<[u32; 3]> {
        if level <= PLANE_COARSEST {
            return None;
        }
        let slot = self.entry(level, cell)?;
        let place = [slot % ATLAS_BRICKS, (slot / ATLAS_BRICKS) % ATLAS_BRICKS, slot / (ATLAS_BRICKS * ATLAS_BRICKS)];
        let padded = u32::try_from(PADDED).unwrap_or(0);
        let mut out = [0u32; 3];
        for a in 0..3 {
            out[a] = place[a] * padded + u32::try_from(cell[a].rem_euclid(BRICK) + 1).ok()?;
        }
        Some(out)
    }

    /// The first word of a fine voxel's record, or `None` where there is none.
    #[must_use]
    pub fn record(&self, level: u8, cell: [i32; 3]) -> Option<usize> {
        if level > PLANE_COARSEST {
            return None;
        }
        let brick = usize::try_from(self.entry(level, cell)?).ok()? * FINE_HEADER;
        let local = cell.map(|c| c.rem_euclid(BRICK));
        let bit = usize::try_from(local[0] + BRICK * local[1] + BRICK * BRICK * local[2]).ok()?;
        let header = self.fine_bricks.get(brick..brick + FINE_HEADER)?;
        let (word, mask) = (bit / 32, 1u32 << (bit % 32));
        if header[1 + word] & mask == 0 {
            return None;
        }
        let before = header[17 + word] + (header[1 + word] & (mask - 1)).count_ones();
        Some(usize::try_from(header[0] + before).ok()? * FINE_RECORD)
    }

    /// The area and light one way's face of a voxel holds, as the GPU reads
    /// them (half floats), or `None` where no brick or record is.
    #[must_use]
    pub fn face(&self, level: u8, cell: [i32; 3], way: usize) -> Option<(f32, V3)> {
        if level <= PLANE_COARSEST {
            let r = self.record(level, cell)?;
            let (red, green) = unpack_half2(self.fine_voxels[r + 2 * way]);
            let (blue, area) = unpack_half2(self.fine_voxels[r + 2 * way + 1]);
            return Some((area, [red, green, blue]));
        }
        let t = self.texel(level, cell)?;
        let i = self.flat(t)? * 4;
        let f = self.faces.get(way)?;
        Some((half_to_f32(f[i + 3]), [half_to_f32(f[i]), half_to_f32(f[i + 1]), half_to_f32(f[i + 2])]))
    }

    fn flat(&self, t: [u32; 3]) -> Option<usize> {
        usize::try_from((t[2] * self.atlas[1] + t[1]) * self.atlas[0] + t[0]).ok()
    }
}

fn pack_half2(a: f32, b: f32) -> u32 {
    u32::from(f32_to_half(a)) | (u32::from(f32_to_half(b)) << 16)
}

fn unpack_half2(w: u32) -> (f32, f32) {
    (half_to_f32(u16::try_from(w & 0xffff).unwrap_or(0)), half_to_f32(u16::try_from(w >> 16).unwrap_or(0)))
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

fn cell_of(p: V3, size: f32) -> [i32; 3] {
    p.map(|c| floor_i32(c / size))
}

/// The points of a triangle, `step` apart, each with its share of the area.
fn sample_triangle(positions: [V3; 3], step: f32, mut emit: impl FnMut(V3, f32)) {
    let [corner, second, third] = positions;
    let (e1, e2) = (sub(second, corner), sub(third, corner));
    let area = 0.5 * dot(cross(e1, e2), cross(e1, e2)).sqrt();
    if area <= 0.0 {
        return;
    }
    let count = |e: V3| -> u32 {
        let n = (dot(e, e).sqrt() / step).ceil().max(1.0);
        // `n` is whole and at least 1; past u16 the triangle is 4 km long.
        u32::from(u16::try_from(half_floor(n)).unwrap_or(u16::MAX))
    };
    let (n1, n2) = (count(e1), count(e2));
    let mut made = Vec::new();
    for i in 0..n1 {
        for j in 0..n2 {
            let u = (f32::from(u16::try_from(i).unwrap_or(0)) + 0.5) / f32::from(u16::try_from(n1).unwrap_or(1));
            let v = (f32::from(u16::try_from(j).unwrap_or(0)) + 0.5) / f32::from(u16::try_from(n2).unwrap_or(1));
            if u + v <= 1.0 {
                made.push(add(corner, add(scale(e1, u), scale(e2, v))));
            }
        }
    }
    let each = area / f32::from(u16::try_from(made.len().max(1)).unwrap_or(u16::MAX));
    for at in made {
        emit(at, each);
    }
}

/// A whole non-negative `f32` below 65536 as `i32`, with no lossy cast.
fn half_floor(v: f32) -> i32 {
    floor_i32(v.clamp(0.0, 65_535.0))
}

/// The scene's surfaces as points for the field's shape, built on every core:
/// as [`surface_points`], but a point's albedo is not read (it is 0), so only
/// a triangle whose material has holes casts the short ray, which keeps a
/// point only where the texel is not a hole. A solid triangle's points are
/// all kept.
#[must_use]
pub fn surface_shape_points(scene: &Scene, gltf: Option<&GltfSurface>) -> Vec<FieldPoint> {
    shape_points(scene, gltf, 0.5 * voxel_size(FIELD_FINEST), None)
}

/// Whether a triangle's box meets `bounds` (none: every triangle does).
fn meets(positions: &[V3; 3], bounds: Option<(V3, V3)>) -> bool {
    let Some((lo, hi)) = bounds else { return true };
    (0..3).all(|a| {
        let low = positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
        let high = positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max);
        high >= lo[a] && low <= hi[a]
    })
}

/// As [`surface_shape_points`], with points `step` apart, from the triangles
/// whose box meets `bounds` only.
fn shape_points(scene: &Scene, gltf: Option<&GltfSurface>, step: f32, bounds: Option<(V3, V3)>) -> Vec<FieldPoint> {
    let mut points = Vec::new();
    let mesh = SceneMesh::from_scene(scene);
    for tri in mesh.triangles.iter().filter(|t| meets(&t.positions, bounds)) {
        let normal = normalize(tri.normals[0]);
        sample_triangle(tri.positions, step, |at, each| {
            points.push((at, normal, each, [0.0; 3]));
            if tri.two_sided {
                points.push((at, scale(normal, -1.0), each, [0.0; 3]));
            }
        });
    }
    if let Some(model) = gltf {
        let triangles: Vec<_> = model.triangles().filter(|t| meets(&t.positions, bounds)).collect();
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let chunk = triangles.len().div_ceil(threads).max(1);
        let parts: Vec<Vec<FieldPoint>> = std::thread::scope(|scope| {
            let handles: Vec<_> = triangles
                .chunks(chunk)
                .map(|tris| {
                    scope.spawn(move || {
                        let mut out = Vec::new();
                        for tri in tris {
                            let [a, b, c] = tri.positions;
                            let mut normal = normalize(cross(sub(b, a), sub(c, a)));
                            if dot(normal, tri.normals[0]) < 0.0 {
                                normal = scale(normal, -1.0);
                            }
                            let sides: &[f32] = if model.double_sided(tri.material) { &[1.0, -1.0] } else { &[1.0] };
                            let holed = model.holed(tri.material);
                            sample_triangle(tri.positions, step, |at, each| {
                                for side in sides.iter().map(|k| scale(normal, *k)) {
                                    if !holed || model.trace(add(at, scale(side, 0.01)), scale(side, -1.0), 0.02).is_some() {
                                        out.push((at, side, each, [0.0; 3]));
                                    }
                                }
                            });
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        points.reserve(parts.iter().map(Vec::len).sum());
        for part in parts {
            points.extend(part);
        }
    }
    points.retain(|(at, normal, _, _)| !(normal[1] < -0.9 && at[1].abs() < 0.01));
    points
}

/// The scene's surfaces as points: the analytic shapes and, when given, a
/// glTF model's triangles (each point's albedo read by a short ray into it).
/// The analytic albedo is scaled by `paint`.
#[must_use]
pub fn surface_points(scene: &Scene, gltf: Option<&GltfSurface>, paint: f32) -> Vec<FieldPoint> {
    let step = 0.5 * voxel_size(FIELD_FINEST);
    let mut points = Vec::new();
    let mesh = SceneMesh::from_scene(scene);
    for tri in &mesh.triangles {
        let normal = normalize(tri.normals[0]);
        let albedo = scale(mesh.materials.get(usize::try_from(tri.material).unwrap_or(usize::MAX)).map_or([0.0; 3], |m| m.albedo), paint);
        sample_triangle(tri.positions, step, |at, each| {
            points.push((at, normal, each, albedo));
            if tri.two_sided {
                points.push((at, scale(normal, -1.0), each, albedo));
            }
        });
    }
    if let Some(model) = gltf {
        for tri in model.triangles() {
            let [a, b, c] = tri.positions;
            let mut normal = normalize(cross(sub(b, a), sub(c, a)));
            if dot(normal, tri.normals[0]) < 0.0 {
                normal = scale(normal, -1.0);
            }
            let sides: &[f32] = if model.double_sided(tri.material) { &[1.0, -1.0] } else { &[1.0] };
            sample_triangle(tri.positions, step, |at, each| {
                for side in sides.iter().map(|k| scale(normal, *k)) {
                    let from = add(at, scale(side, 0.01));
                    if let Some(hit) = model.trace(from, scale(side, -1.0), 0.02) {
                        points.push((at, side, each, hit.albedo));
                    }
                }
            });
        }
    }
    // The analytic ground's underside faces empty space under the world.
    points.retain(|(at, normal, _, _)| !(normal[1] < -0.9 && at[1].abs() < 0.01));
    points
}

/// A fast hasher for voxel keys (a multiply and a rotate a word; the keys are
/// small integers, so `SipHash`'s guard against chosen keys buys nothing here).
#[derive(Default)]
struct CellHasher(u64);

impl Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(u64::from(*b));
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.write_u64(u64::from(i));
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(u64::from(i));
    }
    fn write_i32(&mut self, i: i32) {
        self.write_u32(i.cast_unsigned());
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type Cells = HashMap<(u8, [i32; 3]), Voxel, BuildHasherDefault<CellHasher>>;

/// Tasks a fine level's voxels are split over in [`FieldVoxels::from_surfaces`].
const FINE_SPLIT: u64 = 4;

/// The field's voxels, before bricking.
#[derive(Default)]
pub struct FieldVoxels {
    cells: Cells,
    points: usize,
    /// Each level's clip volume ([`FieldLevel::clip`]).
    clips: [[i32; 4]; FIELD_LEVELS],
}

/// One surface point's share of a voxel (`add_surface` and `from_surfaces`).
fn accumulate(voxel: &mut Voxel, point: V3, normal: V3, area: f32, radiance: V3) {
    for (d, axis) in AXES.iter().enumerate() {
        let facing = dot(normal, *axis).max(0.0) * area;
        voxel.area[d] += facing;
        voxel.light[d] = add(voxel.light[d], scale(radiance, facing));
        voxel.at[d] = add(voxel.at[d], scale(point, facing));
        if facing > 0.0 {
            // A point stands for the patch about it, `area` square: the box
            // covers that patch, so it does not shrink with sparser points.
            let reach = 0.5 * area.sqrt();
            for a in 0..3 {
                voxel.lo[d][a] = voxel.lo[d][a].min(point[a] - reach);
                voxel.hi[d][a] = voxel.hi[d][a].max(point[a] + reach);
            }
        }
    }
}

/// Add voxel `from` into `into`, as if its points had been filed there.
fn merge_voxel(into: &mut Voxel, from: &Voxel) {
    for d in 0..6 {
        into.area[d] += from.area[d];
        into.light[d] = add(into.light[d], from.light[d]);
        into.at[d] = add(into.at[d], from.at[d]);
        for a in 0..3 {
            into.lo[d][a] = into.lo[d][a].min(from.lo[d][a]);
            into.hi[d][a] = into.hi[d][a].max(from.hi[d][a]);
        }
    }
    into.lamp_area += from.lamp_area;
    into.lamp_light = add(into.lamp_light, from.lamp_light);
}

/// The voxel a surface point is filed in at `level`: the one a quarter of a
/// voxel (at most 6.25 cm) behind it.
fn filed_cell(point: V3, normal: V3, level: u8) -> [i32; 3] {
    let size = voxel_size(level);
    cell_of(sub(point, scale(normal, 0.25 * size.min(0.25))), size)
}

fn split_of(cell: [i32; 3]) -> u64 {
    let mut h = CellHasher::default();
    for c in cell {
        h.write_i32(c);
    }
    h.finish() % FINE_SPLIT
}

/// The clip volume of `level` about `eye`: [`CLIP_CELLS`] voxels across,
/// its corner on a brick boundary.
#[must_use]
pub fn clip_around(eye: V3, level: u8) -> [i32; 4] {
    let centre = cell_of(eye, voxel_size(level));
    let corner = centre.map(|c| (c - CLIP_CELLS / 2).div_euclid(BRICK) * BRICK);
    [corner[0], corner[1], corner[2], CLIP_CELLS]
}

/// Whether `cell` is inside clip volume `clip` (a side of 0 holds every cell).
#[must_use]
pub fn clip_holds(clip: [i32; 4], cell: [i32; 3]) -> bool {
    clip[3] == 0 || (0..3).all(|a| cell[a] >= clip[a] && cell[a] < clip[a] + clip[3])
}

/// The voxels of `points` at `levels`, each kept only inside `clip`, on every
/// core: [`FINE_SPLIT`] tasks for the finest of `levels` and one for each
/// other, each reading every point in order (the sums are the serial ones).
fn file_levels(points: &[FieldPoint], levels: std::ops::RangeInclusive<u8>, clip: Option<[i32; 4]>) -> Cells {
    // The finest level of a set holds most of its voxels: split it.
    let first = *levels.start();
    let tasks: Vec<(u8, u64)> = levels
        .flat_map(|level| {
            let parts = if level == first { FINE_SPLIT } else { 1 };
            (0..parts).map(move |part| (level, part))
        })
        .collect();
    let parts: Vec<Cells> = std::thread::scope(|scope| {
        let handles: Vec<_> = tasks
            .iter()
            .map(|&(level, part)| {
                scope.spawn(move || {
                    let split = level == first;
                    let mut cells = Cells::default();
                    for (at, normal, area, _) in points {
                        let cell = filed_cell(*at, *normal, level);
                        if split && split_of(cell) != part {
                            continue;
                        }
                        if clip.is_some_and(|c| !clip_holds(c, cell)) {
                            continue;
                        }
                        accumulate(cells.entry((level, cell)).or_default(), *at, *normal, *area, [0.0; 3]);
                    }
                    cells
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let mut cells = Cells::default();
    cells.reserve(parts.iter().map(HashMap::len).sum());
    for part in parts {
        cells.extend(part);
    }
    cells
}

impl FieldVoxels {
    /// A surface point that sends out `radiance`.
    pub fn add_surface(&mut self, point: V3, normal: V3, area: f32, radiance: V3) {
        self.points += 1;
        for level in FIELD_FINEST..=FIELD_COARSEST {
            let voxel = self.cells.entry((level, filed_cell(point, normal, level))).or_default();
            accumulate(voxel, point, normal, area, radiance);
        }
    }

    /// The voxels of these surface points (none sending light), built on
    /// every core: one task a coarse level and [`FINE_SPLIT`] a fine one, each
    /// reading every point in order and keeping its own voxels, so the sums
    /// are the ones [`FieldVoxels::add_surface`] makes, bit for bit.
    #[must_use]
    pub fn from_surfaces(points: &[FieldPoint]) -> FieldVoxels {
        let tasks: Vec<(u8, u64)> = (FIELD_FINEST..=FIELD_COARSEST)
            .flat_map(|level| {
                let parts = if level <= PLANE_COARSEST { FINE_SPLIT } else { 1 };
                (0..parts).map(move |part| (level, part))
            })
            .collect();
        let parts: Vec<Cells> = std::thread::scope(|scope| {
            let handles: Vec<_> = tasks
                .iter()
                .map(|&(level, part)| {
                    scope.spawn(move || {
                        let split = level <= PLANE_COARSEST;
                        let mut cells = Cells::default();
                        for (at, normal, area, _) in points {
                            let cell = filed_cell(*at, *normal, level);
                            if split && split_of(cell) != part {
                                continue;
                            }
                            accumulate(cells.entry((level, cell)).or_default(), *at, *normal, *area, [0.0; 3]);
                        }
                        cells
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut cells = Cells::default();
        cells.reserve(parts.iter().map(HashMap::len).sum());
        for part in parts {
            cells.extend(part);
        }
        FieldVoxels { cells, points: points.len(), clips: [[0; 4]; FIELD_LEVELS] }
    }

    /// The field around `eye`, sampled level by level: the coarse levels
    /// ([`WHOLE_FINEST`] up) from points half a [`WHOLE_FINEST`] voxel apart
    /// over the whole scene, and each finer level from points half its voxel
    /// apart inside its clip volume ([`CLIP_CELLS`] across, about the eye).
    /// A level's area is the same as from the finest points; its voxels'
    /// seats are the mean of fewer points. Without `fine` the fine levels
    /// hold nothing ([`NO_CLIP`]): the coarse levels alone, quick to build,
    /// stand in until the fine ones are.
    #[must_use]
    pub fn around_eye(scene: &Scene, gltf: Option<&GltfSurface>, eye: V3, fine: bool) -> FieldVoxels {
        let mut clips = [[0; 4]; FIELD_LEVELS];
        for level in FIELD_FINEST..WHOLE_FINEST {
            clips[usize::from(level - FIELD_FINEST)] = if fine { clip_around(eye, level) } else { NO_CLIP };
        }
        // The whole levels and each clip level at once: each samples and
        // files on every core, and together they keep the cores busy while
        // one of them samples.
        let groups: Vec<(Cells, usize)> = std::thread::scope(|scope| {
            let whole = scope.spawn(|| {
                let points = shape_points(scene, gltf, 0.5 * voxel_size(WHOLE_FINEST), None);
                let mut cells = file_levels(&points, WHOLE_FINEST..=WHOLE_FINEST, None);
                // From the finest whole level up a point is filed the same
                // distance behind it (filed_cell), so each coarser voxel is the
                // sum of its eight children: built from them, not from the
                // points again.
                let mut below: Vec<([i32; 3], Voxel)> = cells.iter().map(|((_, c), v)| (*c, *v)).collect();
                for level in WHOLE_FINEST + 1..=FIELD_COARSEST {
                    let mut up = Cells::default();
                    for (cell, voxel) in &below {
                        merge_voxel(up.entry((level, cell.map(|c| c.div_euclid(2)))).or_default(), voxel);
                    }
                    below = up.iter().map(|((_, c), v)| (*c, *v)).collect();
                    cells.extend(up);
                }
                (cells, points.len())
            });
            let fine: Vec<_> = (FIELD_FINEST..WHOLE_FINEST)
                .filter(|_| fine)
                .map(|level| {
                    let clip = clips[usize::from(level - FIELD_FINEST)];
                    scope.spawn(move || {
                        let size = voxel_size(level);
                        let lo = [0, 1, 2].map(|a| cell_low(clip[a], size) - size);
                        let hi = [0, 1, 2].map(|a| cell_low(clip[a] + clip[3], size) + size);
                        let points = shape_points(scene, gltf, 0.5 * size, Some((lo, hi)));
                        (file_levels(&points, level..=level, Some(clip)), points.len())
                    })
                })
                .collect();
            std::iter::once(whole).chain(fine).filter_map(|h| h.join().ok()).collect()
        });
        let mut cells = Cells::default();
        cells.reserve(groups.iter().map(|(c, _)| c.len()).sum());
        let mut points = 0;
        for (part, n) in groups {
            cells.extend(part);
            points += n;
        }
        FieldVoxels { cells, points, clips }
    }

    /// A lamp: a sphere of `radius` sending `glow` every way, its disc
    /// shared over points of the sphere.
    pub fn add_lamp(&mut self, centre: V3, radius: f32, glow: V3) {
        let share = PI * radius * radius / f32::from(u16::try_from(LAMP_POINTS).unwrap_or(1));
        for i in 0..LAMP_POINTS {
            let (u, v) = r2(i);
            let at = add(centre, scale(decode_dir(u, v), radius));
            for level in FIELD_FINEST..=FIELD_COARSEST {
                let voxel = self.cells.entry((level, cell_of(at, voxel_size(level)))).or_default();
                voxel.lamp_area += share;
                voxel.lamp_light = add(voxel.lamp_light, scale(glow, share));
            }
        }
    }

    /// The scene's point lamps as glowing spheres of `radius` (the engine's
    /// lamp unit over the sphere's disc).
    pub fn add_scene_lamps(&mut self, scene: &Scene, radius: f32) {
        for i in 0..scene.light_len() {
            let l = scene.light(i);
            if l.direction.x != 0.0 || l.direction.y != 0.0 || l.direction.z != 0.0 {
                continue;
            }
            let glow = scale(l.color, LAMP_UNIT / (PI * radius * radius));
            self.add_lamp([l.position.x, l.position.y, l.position.z], radius, glow);
        }
    }

    /// For each level: its voxels and the area they hold (all six ways).
    #[must_use]
    pub fn level_totals(&self) -> [(usize, f64); FIELD_LEVELS] {
        let mut out = [(0usize, 0.0f64); FIELD_LEVELS];
        for ((level, _), voxel) in &self.cells {
            let k = usize::from(level - FIELD_FINEST);
            out[k].0 += 1;
            out[k].1 += voxel.area.iter().map(|a| f64::from(*a)).sum::<f64>();
        }
        out
    }

    #[must_use]
    pub fn voxel_count(&self) -> usize {
        self.cells.len()
    }

    /// The bricks the GPU reads.
    #[must_use]
    pub fn brick(&self) -> ConeField {
        // The cells with a surface, and their neighbours; each level's bricks
        // (a coarse brick also where a voxel is one cell outside it, so its
        // apron is whole). One task a level.
        type CellSet = HashSet<[i32; 3], BuildHasherDefault<CellHasher>>;
        let sets: Vec<(CellSet, CellSet)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (FIELD_FINEST..=FIELD_COARSEST)
                .map(|level| {
                    scope.spawn(move || {
                        let mut near = CellSet::default();
                        let mut bricks = CellSet::default();
                        for ((l, cell), voxel) in &self.cells {
                            if *l != level {
                                continue;
                            }
                            let surface = voxel.area.iter().any(|a| *a > 0.0);
                            if !surface && voxel.lamp_area <= 0.0 {
                                continue;
                            }
                            if level <= PLANE_COARSEST {
                                bricks.insert(cell.map(|v| v.div_euclid(BRICK)));
                                continue;
                            }
                            for dz in -1..=1 {
                                for dy in -1..=1 {
                                    for dx in -1..=1 {
                                        let c = [cell[0] + dx, cell[1] + dy, cell[2] + dz];
                                        if surface {
                                            near.insert(c);
                                        }
                                        bricks.insert(c.map(|v| v.div_euclid(BRICK)));
                                    }
                                }
                            }
                        }
                        (near, bricks)
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let (near, bricks): (Vec<CellSet>, Vec<CellSet>) = sets.into_iter().unzip();
        // Each level's table over the box of its bricks.
        let mut levels = [FieldLevel::default(); FIELD_LEVELS];
        let mut table: Vec<u32> = Vec::new();
        let (mut fine, mut coarse) = (0u32, 0u32);
        let mut placed: Vec<(u8, [i32; 3], u32)> = Vec::new();
        for (k, set) in bricks.iter().enumerate() {
            let level = FIELD_FINEST + u8::try_from(k).unwrap_or(0);
            let offset = u32::try_from(table.len()).unwrap_or(u32::MAX);
            if set.is_empty() {
                levels[k] = FieldLevel { origin: [0; 3], dims: [0; 3], offset, clip: self.clips[k] };
                continue;
            }
            let lo = set.iter().fold([i32::MAX; 3], |m, b| [m[0].min(b[0]), m[1].min(b[1]), m[2].min(b[2])]);
            let hi = set.iter().fold([i32::MIN; 3], |m, b| [m[0].max(b[0]), m[1].max(b[1]), m[2].max(b[2])]);
            let dims = [0, 1, 2].map(|a| u32::try_from(hi[a] - lo[a] + 1).unwrap_or(0));
            levels[k] = FieldLevel { origin: lo, dims, offset, clip: self.clips[k] };
            table.resize(table.len() + usize::try_from(dims[0] * dims[1] * dims[2]).unwrap_or(0), NO_BRICK);
            let mut sorted: Vec<[i32; 3]> = set.iter().copied().collect();
            sorted.sort_unstable();
            for b in sorted {
                let rel = [0, 1, 2].map(|a| u32::try_from(b[a] - lo[a]).unwrap_or(0));
                let index = offset + (rel[2] * dims[1] + rel[1]) * dims[0] + rel[0];
                let slot = if level <= PLANE_COARSEST { &mut fine } else { &mut coarse };
                if let Some(entry) = table.get_mut(usize::try_from(index).unwrap_or(usize::MAX)) {
                    *entry = *slot;
                }
                placed.push((level, b, *slot));
                *slot += 1;
            }
        }
        let padded = u32::try_from(PADDED).unwrap_or(0);
        let slabs = coarse.div_ceil(ATLAS_BRICKS * ATLAS_BRICKS).max(1);
        let atlas = [ATLAS_BRICKS * padded, ATLAS_BRICKS * padded, slabs * padded];
        let texels = usize::try_from(atlas[0] * atlas[1] * atlas[2]).unwrap_or(0);
        let mut field = ConeField {
            levels,
            table,
            fine_bricks: vec![0u32; usize::try_from(fine).unwrap_or(0) * FINE_HEADER],
            fine_voxels: Vec::new(),
            coarse_bricks: coarse,
            atlas,
            faces: std::array::from_fn(|_| vec![0u16; texels * 4]),
            occupancy: vec![0u8; texels],
            coarse_seats: vec![0u32; texels],
            voxels: self.cells.len(),
            points: self.points,
        };
        // The coarse texels are looked up on every core, a share of the
        // bricks each, then written in.
        let coarse_placed: Vec<(u8, [i32; 3], u32)> = placed.iter().copied().filter(|(l, _, _)| *l > PLANE_COARSEST).collect();
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let chunk = coarse_placed.len().div_ceil(threads).max(1);
        let atlas = field.atlas;
        let flat = |t: [u32; 3]| usize::try_from((t[2] * atlas[1] + t[1]) * atlas[0] + t[0]).ok();
        let near = &near;
        let texels: Vec<Vec<(usize, bool, Option<(Voxel, u32)>)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = coarse_placed
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || {
                        let mut out = Vec::new();
                        for &(level, b, s) in part {
                            let place = [s % ATLAS_BRICKS, (s / ATLAS_BRICKS) % ATLAS_BRICKS, s / (ATLAS_BRICKS * ATLAS_BRICKS)];
                            let near = &near[usize::from(level - FIELD_FINEST)];
                            for z in 0..PADDED {
                                for y in 0..PADDED {
                                    for x in 0..PADDED {
                                        let cell = [b[0] * BRICK + x - 1, b[1] * BRICK + y - 1, b[2] * BRICK + z - 1];
                                        let local = [x, y, z].map(|v| u32::try_from(v).unwrap_or(0));
                                        let Some(i) = flat([0, 1, 2].map(|a| place[a] * padded + local[a])) else { continue };
                                        let voxel = self.cells.get(&(level, cell)).map(|v| (*v, seat_word(v, cell, voxel_size(level))));
                                        let occupied = near.contains(&cell);
                                        if occupied || voxel.is_some() {
                                            out.push((i, occupied, voxel));
                                        }
                                    }
                                }
                            }
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for (i, occupied, voxel) in texels.into_iter().flatten() {
            if occupied {
                field.occupancy[i] = u8::MAX;
            }
            if let Some((voxel, seat)) = voxel {
                coarse_texel(&mut field, i, &voxel);
                field.coarse_seats[i] = seat;
            }
        }
        for (level, b, s) in placed {
            if level <= PLANE_COARSEST {
                self.fill_fine(&mut field, level, b, s);
            }
        }
        field
    }
}

impl FieldVoxels {
    /// A fine brick's header at `slot`, and its voxels' records appended.
    fn fill_fine(&self, field: &mut ConeField, level: u8, brick: [i32; 3], slot: u32) {
        let size = voxel_size(level);
        let h = usize::try_from(slot).unwrap_or(0) * FINE_HEADER;
        field.fine_bricks[h] = u32::try_from(field.fine_voxels.len() / FINE_RECORD).unwrap_or(u32::MAX);
        let mut count = 0u32;
        for word in 0..16usize {
            field.fine_bricks[h + 17 + word] = count;
            let mut mask = 0u32;
            for bit in 0..32usize {
                let index = i32::try_from(word * 32 + bit).unwrap_or(0);
                let local = [index % BRICK, (index / BRICK) % BRICK, index / (BRICK * BRICK)];
                let cell = [0, 1, 2].map(|a| brick[a] * BRICK + local[a]);
                if let Some(voxel) = self.cells.get(&(level, cell)) {
                    mask |= 1 << bit;
                    count += 1;
                    fine_record(&mut field.fine_voxels, voxel, cell, size);
                }
            }
            field.fine_bricks[h + 1 + word] = mask;
        }
    }
}

/// A coarse voxel's texels in the face atlases at flat texel `i`.
fn coarse_texel(field: &mut ConeField, i: usize, voxel: &Voxel) {
    for d in 0..6 {
        let light = voxel.light[d];
        field.faces[d][i * 4..i * 4 + 4].copy_from_slice(&[f32_to_half(light[0]), f32_to_half(light[1]), f32_to_half(light[2]), f32_to_half(voxel.area[d])]);
    }
    field.faces[6][i * 4..i * 4 + 4].copy_from_slice(&[
        f32_to_half(voxel.lamp_light[0]),
        f32_to_half(voxel.lamp_light[1]),
        f32_to_half(voxel.lamp_light[2]),
        f32_to_half(voxel.lamp_area),
    ]);
}

/// A fine voxel's record, appended to `out` ([`FINE_RECORD`] words).
fn fine_record(out: &mut Vec<u32>, voxel: &Voxel, cell: [i32; 3], size: f32) {
    let low = cell.map(|c| f32::from(i16::try_from(c.clamp(-32_768, 32_767)).unwrap_or(0)) * size);
    for d in 0..6 {
        let l = voxel.light[d];
        out.push(pack_half2(l[0], l[1]));
        out.push(pack_half2(l[2], voxel.area[d]));
    }
    for d in 0..6 {
        if voxel.area[d] <= 0.0 {
            out.extend([0, 0, 0]);
            continue;
        }
        let (ax, u, v) = (d / 2, (d / 2 + 1) % 3, (d / 2 + 2) % 3);
        let rel = |x: f32, a: usize| (x - low[a]) / size;
        let centroid = voxel.at[d][ax] / voxel.area[d];
        out.push(pack_half2(rel(centroid, ax), rel(voxel.lo[d][u], u)));
        out.push(pack_half2(rel(voxel.hi[d][u], u), rel(voxel.lo[d][v], v)));
        out.push(pack_half2(rel(voxel.hi[d][v], v), 0.0));
    }
    out.push(pack_half2(voxel.lamp_light[0], voxel.lamp_light[1]));
    out.push(pack_half2(voxel.lamp_light[2], voxel.lamp_area));
    out.push(seat_word(voxel, cell, size));
}

/// A voxel's seat: the area-weighted mean of the surface points filed in it,
/// a fixed place in the world that a probe on this voxel sits on whatever
/// the camera sees. Bit 31 set when the voxel has surface; bits 0..29 the
/// point in 10 bits an axis over the voxel's cell from half a voxel below to
/// half a voxel past (a point is filed a quarter voxel behind its surface, so
/// it can sit just outside the cell). 0 when the voxel has no surface.
#[must_use]
pub fn seat_word_of(mean: V3, cell: [i32; 3], size: f32) -> u32 {
    let mut word = 0x8000_0000u32;
    for a in 0..3 {
        let rel = (mean[a] - cell_low(cell[a], size)) / size;
        let q = ((rel + 0.5) * 0.5 * 1023.0).round().clamp(0.0, 1023.0);
        word |= (q as u32) << (10 * a);
    }
    word
}

fn cell_low(c: i32, size: f32) -> f32 {
    f32::from(i16::try_from(c.clamp(-32_768, 32_767)).unwrap_or(0)) * size
}

fn seat_word(voxel: &Voxel, cell: [i32; 3], size: f32) -> u32 {
    let total: f32 = voxel.area.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut mean = [0.0f32; 3];
    for d in 0..6 {
        mean = add(mean, voxel.at[d]);
    }
    seat_word_of(scale(mean, 1.0 / total), cell, size)
}

/// The point a seat word stands for, or `None` for 0.
#[must_use]
pub fn seat_point(word: u32, cell: [i32; 3], size: f32) -> Option<V3> {
    if word & 0x8000_0000 == 0 {
        return None;
    }
    let mut p = [0.0f32; 3];
    for a in 0..3 {
        let q = ((word >> (10 * a)) & 1023) as f32;
        p[a] = cell_low(cell[a], size) + (q / 1023.0 * 2.0 - 0.5) * size;
    }
    Some(p)
}

/// The plastic sequence's `index`th point of the unit square.
fn r2(index: u32) -> (f32, f32) {
    const G: f32 = 1.324_718;
    let i = f32::from(u16::try_from(index).unwrap_or(u16::MAX));
    ((0.5 + i / G).fract(), (0.5 + i / (G * G)).fract())
}

fn decode_dir(u: f32, v: f32) -> V3 {
    let phi = u * 2.0 * PI;
    let z = 2.0 * v - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    [r * phi.cos(), r * phi.sin(), z]
}

/// Half-float bits of `x`, rounded toward zero; too large is the largest half.
#[must_use]
pub fn f32_to_half(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = u16::try_from((bits >> 16) & 0x8000).unwrap_or(0);
    let exponent = i64::from((bits >> 23) & 0xff) - 127 + 15;
    let mantissa = bits & 0x007f_ffff;
    if exponent <= 0 {
        return sign;
    }
    if exponent >= 31 {
        return sign | 0x7bff;
    }
    let e = u16::try_from(exponent).unwrap_or(0) << 10;
    sign | e | u16::try_from(mantissa >> 13).unwrap_or(0)
}

/// The `f32` of half-float bits.
#[must_use]
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((h >> 10) & 0x1f);
    let mantissa = f32::from(h & 0x03ff);
    if exponent == 0 {
        return sign * mantissa / 1024.0 * 2f32.powi(-14);
    }
    sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15)
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::*;

    /// A square wall of 2 m in the x = 1 plane, facing +x, sampled as the
    /// scene's surfaces are.
    fn wall() -> FieldVoxels {
        let mut v = FieldVoxels::default();
        let step = 0.5 * voxel_size(FIELD_FINEST);
        for tri in [[[1.0, 0.0, 0.0], [1.0, 2.0, 0.0], [1.0, 2.0, 2.0]], [[1.0, 0.0, 0.0], [1.0, 2.0, 2.0], [1.0, 0.0, 2.0]]] {
            sample_triangle(tri, step, |at, each| v.add_surface(at, [1.0, 0.0, 0.0], each, [0.5, 0.25, 0.125]));
        }
        v
    }

    #[test]
    fn a_wall_fills_its_voxels_through_the_bricks() {
        let field = wall().brick();
        // Inside the wall, every level: the +x way's area is the voxel's face.
        // The point is off the diagonal where the two triangles' samples meet.
        for level in FIELD_FINEST..=4 {
            let size = voxel_size(level);
            let cell = cell_of([1.0 - 0.1 * size, 1.45, 0.55], size);
            let (area, light) = field.face(level, cell, 0).expect("a brick at the wall");
            let fill = area / (size * size);
            assert!((0.8..=1.2).contains(&fill), "level {level}: fill {fill}");
            let radiance = light[0] / area;
            assert!((radiance - 0.5).abs() < 0.01, "level {level}: radiance {radiance}");
            // Nothing faces -x.
            assert_eq!(field.face(level, cell, 1).map(|f| f.0), Some(0.0));
        }
        // Far from the wall there is no brick.
        assert!(field.face(FIELD_FINEST, cell_of([40.0, 1.0, 1.0], voxel_size(FIELD_FINEST)), 0).is_none());
    }

    #[test]
    fn the_parallel_build_files_every_point_as_the_serial_one_does() {
        let mut points = Vec::new();
        let step = 0.5 * voxel_size(FIELD_FINEST);
        for tri in [[[1.0, 0.0, 0.0], [1.0, 2.0, 0.0], [1.0, 2.0, 2.0]], [[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 0.0, 3.0]]] {
            let normal = normalize(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])));
            sample_triangle(tri, step, |at, each| points.push((at, normal, each, [0.0; 3])));
        }
        let mut serial = FieldVoxels::default();
        for (at, normal, area, _) in &points {
            serial.add_surface(*at, *normal, *area, [0.0; 3]);
        }
        let parallel = FieldVoxels::from_surfaces(&points);
        assert_eq!(serial.cells.len(), parallel.cells.len());
        for (key, voxel) in &serial.cells {
            let other = parallel.cells.get(key).expect("the same voxel");
            assert_eq!(voxel.area, other.area, "{key:?}");
            assert_eq!(voxel.at, other.at, "{key:?}");
            assert_eq!(voxel.lo, other.lo, "{key:?}");
            assert_eq!(voxel.hi, other.hi, "{key:?}");
        }
    }

    /// The field about the eye covers a floor and a wall as the whole-scene
    /// build does: each level holds the same area, though the levels are
    /// sampled at their own spacing and the fine ones only in their clip
    /// volumes. Built with the coarse levels alone, the clip levels hold
    /// nothing and the whole levels the same.
    #[test]
    fn the_field_about_the_eye_covers_a_wall_as_the_whole_build_does() {
        let scene = genos_scene::Scene::from_lists(
            genos_scene::Floor { position: genos_scene::Vec3::ZERO, half_x: 3.0, half_z: 3.0, color: [0.8; 3] },
            vec![genos_scene::Wall {
                position: genos_scene::Vec3::new(0.0, 0.0, -1.0),
                half_x: 2.0,
                half_z: 0.1,
                height: 2.0,
                base: 0.0,
                color: [0.8; 3],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            Vec::new(),
            Vec::new(),
        );
        let whole = FieldVoxels::from_surfaces(&surface_shape_points(&scene, None)).level_totals();
        let near = FieldVoxels::around_eye(&scene, None, [0.0, 1.0, 0.0], true).level_totals();
        let coarse = FieldVoxels::around_eye(&scene, None, [0.0, 1.0, 0.0], false).level_totals();
        for k in 0..FIELD_LEVELS {
            let (want, got) = (whole[k].1, near[k].1);
            assert!(want > 0.0, "level {}: the whole build holds no area", k + 2);
            assert!((got - want).abs() <= 1e-3 * want, "level {}: area {got} about the eye, {want} whole", k + 2);
            let level = FIELD_FINEST + u8::try_from(k).unwrap_or(0);
            if level < WHOLE_FINEST {
                assert_eq!(coarse[k].0, 0, "level {level}: the coarse build holds clip voxels");
            } else {
                assert!((coarse[k].1 - want).abs() <= 1e-3 * want, "level {level}: coarse area {}", coarse[k].1);
            }
        }
    }

    #[test]
    fn a_wall_voxels_seat_lies_on_the_wall_at_every_level() {
        let field = wall().brick();
        for level in FIELD_FINEST..=6 {
            let size = voxel_size(level);
            // The voxel the wall's points are filed in: a quarter voxel
            // behind the surface (the probe's cell is found the same way).
            let cell = cell_of([1.0 - 0.25 * size.min(0.25), 1.45, 0.55], size);
            let word = if level <= PLANE_COARSEST {
                let at = field.record(level, cell).expect("a fine record");
                field.fine_voxels[at + FINE_RECORD - 1]
            } else {
                let t = field.texel(level, cell).expect("a coarse texel");
                field.coarse_seats[field.flat(t).expect("texel")]
            };
            let seat = seat_point(word, cell, size).expect("a seat");
            // On the wall plane, inside the voxel's square of the wall.
            assert!((seat[0] - 1.0).abs() < 0.002 * size.max(0.5), "level {level}: x {}", seat[0]);
            for a in [1, 2] {
                let low = cell_low(cell[a], size);
                assert!(seat[a] >= low - 1e-4 && seat[a] <= low + size + 1e-4, "level {level}: axis {a} {}", seat[a]);
            }
        }
        // A voxel with no surface has no seat.
        assert_eq!(seat_point(0, [0, 0, 0], 1.0), None);
    }

    #[test]
    fn the_apron_holds_the_next_bricks_voxels() {
        let field = wall().brick();
        let level = PLANE_COARSEST + 1;
        let size = voxel_size(level);
        // The wall voxel at the high y edge of a brick, read from the brick
        // above through its apron: the same texel values.
        let cell = cell_of([1.0 - 0.1 * size, 7.5 * size, 0.3], size);
        let t = field.texel(level, cell).expect("a brick");
        let above = [cell[0], cell[1] + 1, cell[2]];
        let ta = field.texel(level, above).expect("the brick above");
        // The apron texel under `above` in its brick is one below it in y.
        let apron = [ta[0], ta[1] - 1, ta[2]];
        let i = field.flat(t).expect("texel") * 4;
        let j = field.flat(apron).expect("apron") * 4;
        assert_eq!(field.faces[0][i..i + 4], field.faces[0][j..j + 4]);
        assert!(half_to_f32(field.faces[0][i + 3]) > 0.0);
    }

    #[test]
    fn halves_round_trip_the_values_the_field_holds() {
        for x in [0.0, 0.003_9, 0.25, 1.0, 3.5, 1_000.0] {
            let back = half_to_f32(f32_to_half(x));
            assert!((back - x).abs() <= x * 0.001 + 1.0e-6, "{x} -> {back}");
        }
    }
}

/// One cascade's probe counts from the last frame (`lighting.cone_stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConeCascadeStats {
    /// Probes the hash held (kept plus dropped in compact).
    pub probes: u32,
    /// The cascade's probe table size.
    pub cap: u32,
    /// Probes past the cap: compact gave them no record, so nothing traced them.
    pub dropped_compact: u32,
    /// Pixels whose probe found no hash slot (the slot chain was full).
    pub dropped_insert: u32,
    pub cones_traced: u32,
    pub cones_closed: u32,
    /// Probes whose voxel the field had no surface for, seated at the cell's
    /// centre (a mover, or a level past the field's coarsest).
    pub seat_fallbacks: u32,
}

/// One probe of a cascade as the GPU wrote it last frame (`lighting.cone_probes`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeProbe {
    pub key: u32,
    pub lod: u32,
    pub cell: [i32; 3],
    pub pixels: u32,
    pub seat: [f32; 3],
}
