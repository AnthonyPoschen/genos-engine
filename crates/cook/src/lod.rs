//! Discrete LOD chain for one mesh (geometry LOD stage 1).
//!
//! Four levels, simplified from the imported triangles with `meshopt_simplify`
//! at 50%, 25%, and 12.5%. Level 0 is the import, error 0.
//! The renderer does not link this module's crate dependency: `genos-render`
//! does not depend on `genos-cook`.
//!
//! `meshopt_simplify` reports the max collapse residual (`meshopt_SimplifyErrorAbsolute`,
//! metres). That residual can sit below the distance between the two surfaces.
//! The stored error is a two-sided upper bound. Distance to one onto triangle
//! is convex, so its maximum on a sub-triangle is attained at a vertex, and
//! distance to the whole surface is at most that. The Lipschitz value
//! `distance(centroid) + r` is the other certificate. The cook stores the
//! tighter of the two and splits only where that upper can still rise. A
//! coplanar patch has a second certificate: distance to a plane is convex, so
//! once the onto triangles that meet a region lie in a thin slab and their
//! projections leave nothing when subtracted from the query polygon, the
//! region stops. A duplicate or an overlap cannot be counted twice. A flat
//! sheet therefore stops at the other mesh's triangle
//! size instead of at [`LOD_COVER_STOP_M`]. A per-mesh query budget stops a
//! runaway split and
//! keeps the largest upper still on the heap; that hit is counted by
//! [`lod_cover_budget_hits`]. The simplifier residual is not the bound: it
//! can sit above or below the surface distance. A non-finite sample or
//! residual is not stored as zero.
//!
//! Each primitive is simplified with `LockBorder`, so a shared edge between
//! primitives stays on the imported vertices. An open n×n sheet then keeps its
//! 4n boundary vertices, and a triangulation of that loop has at least 4n−2
//! triangles.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

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
/// Stamped into the cook hash. 5 = the coplanar close uses the offset miter.
const LOD_ERROR_BOUND: u8 = 5;
/// Stop splitting a sub-triangle once its covering radius is at most this, metres.
/// 1/256 is exact in `f32`. Flat sheets close earlier, when one onto triangle
/// already certifies the vertices.
pub const LOD_COVER_STOP_M: f32 = 1.0 / 256.0;
/// Most cover queries one mesh may spend across every level and both directions.
/// Past this the cook keeps the largest upper still on the heap.
pub const LOD_COVER_QUERY_BUDGET: u32 = 2_000_000;
/// Split cap for one lineage. The query budget is what bounds the frontier.
const COVER_DEPTH: u32 = 16;
/// Times the cook stopped early because [`LOD_COVER_QUERY_BUDGET`] was spent.
static COVER_BUDGET_HITS: AtomicU32 = AtomicU32::new(0);

/// How many cooks on this process stored a budget-limited error.
#[must_use]
pub fn lod_cover_budget_hits() -> u32 {
    COVER_BUDGET_HITS.load(AtomicOrdering::Relaxed)
}
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
    hasher.update(&LOD_COVER_QUERY_BUDGET.to_le_bytes());
}

/// The four-level chain. Level 0 is `mesh` unchanged.
#[must_use]
pub fn lod_chain(mesh: &GltfMesh) -> Vec<MeshLod> {
    let mut budget = CoverBudget::full();
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
            let next_error = combined_error(residual, surface_bound(prim, &next, &mut budget));
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
    if budget.limited {
        COVER_BUDGET_HITS.fetch_add(1, AtomicOrdering::Relaxed);
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

struct CoverBudget {
    left: u32,
    limited: bool,
}

impl CoverBudget {
    fn full() -> Self {
        Self {
            left: LOD_COVER_QUERY_BUDGET,
            limited: false,
        }
    }

    fn take(&mut self, count: u32) -> bool {
        if self.left < count {
            self.limited = true;
            false
        } else {
            self.left -= count;
            true
        }
    }
}

/// Upper bound on the distance between the two surfaces, both directions.
/// `None` when a sample, a radius, or a closest-point distance is non-finite.
fn surface_bound(source: &Primitive, level: &Primitive, budget: &mut CoverBudget) -> Option<f32> {
    if same_triangle_set(source, level)? {
        return Some(0.0);
    }
    let fine_to_coarse = directed_bound(source, level, budget)?;
    let coarse_to_fine = directed_bound(level, source, budget)?;
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
/// `upper` bounds the distance from any of its points to `onto`.
/// `closed` means splitting cannot be required: a vertex certificate is
/// already within [`LOD_COVER_STOP_M`] of the centroid distance, or `r` is.
#[derive(Clone, Copy)]
struct Cover {
    corners: [[f32; 3]; 3],
    upper: f32,
    depth: u32,
    id: u64,
    closed: bool,
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

fn directed_bound(from: &Primitive, onto: &Primitive, budget: &mut CoverBudget) -> Option<f32> {
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
    let surface = OntoSurface {
        grid: RefCell::new(None),
        tris: &onto_tris,
        bvh: &bvh,
    };
    let mut scratch = CoverScratch::new(onto_tris.len());
    let loose = pair_diagonal(&from.positions, &onto.positions);
    if !loose.is_finite() {
        return None;
    }
    let mut next_id = 0_u64;
    let mut heap = BinaryHeap::new();
    for corners in from_tris {
        let id = next_id;
        next_id = next_id.saturating_add(1);
        if budget.take(1) {
            heap.push(cover_of(&surface, corners, 0, loose, id, &mut scratch)?);
        } else {
            heap.push(Cover {
                corners,
                upper: loose,
                depth: 0,
                id,
                closed: true,
            });
        }
    }
    while let Some(region) = heap.pop() {
        if region.closed || region.depth >= COVER_DEPTH {
            return region.upper.is_finite().then_some(region.upper);
        }
        // Four children replace this region. Spending fewer would leave a gap.
        if !budget.take(4) {
            return region.upper.is_finite().then_some(region.upper);
        }
        for child in subdivide(region.corners) {
            let id = next_id;
            next_id = next_id.saturating_add(1);
            heap.push(cover_of(
                &surface,
                child,
                region.depth.saturating_add(1),
                loose,
                id,
                &mut scratch,
            )?);
        }
    }
    Some(0.0)
}

struct OntoSurface<'a> {
    tris: &'a [MeshTriangle],
    bvh: &'a Bvh,
    /// Built on the first coplanar-cover query. A mesh that closes from the
    /// vertex certificate never allocates it.
    grid: RefCell<Option<TriGrid>>,
}

struct CoverScratch {
    seen: Vec<u32>,
    stamp: u32,
    hits: Vec<u32>,
}

impl CoverScratch {
    fn new(triangles: usize) -> Self {
        Self {
            seen: vec![0; triangles],
            stamp: 0,
            hits: Vec::new(),
        }
    }
}

fn cover_of(
    onto: &OntoSurface<'_>,
    corners: [[f32; 3]; 3],
    depth: u32,
    loose: f32,
    id: u64,
    scratch: &mut CoverScratch,
) -> Option<Cover> {
    let sample = centroid(corners);
    if sample.iter().any(|axis| !axis.is_finite()) {
        return None;
    }
    let radius = covering_radius(sample, corners)?;
    let (index, _, raw) = onto.bvh.closest(onto.tris, sample, loose)?;
    if !raw.is_finite() {
        return None;
    }
    let distance = padded_distance(raw, sample);
    let lipschitz = f32::next_up(distance + radius);
    let tri = triangle_ref(onto.tris, index)?;
    let mut vertex_upper = vertex_certificate(tri, corners)?;
    if vertex_upper > distance + LOD_COVER_STOP_M {
        for corner in corners {
            let (corner_index, _, corner_raw) = onto.bvh.closest(onto.tris, corner, loose)?;
            if !corner_raw.is_finite() {
                return None;
            }
            if corner_index == index {
                continue;
            }
            let corner_tri = triangle_ref(onto.tris, corner_index)?;
            vertex_upper = vertex_upper.min(vertex_certificate(corner_tri, corners)?);
        }
    }
    let mut upper = lipschitz.min(vertex_upper);
    let vertex_tight = vertex_upper <= distance + LOD_COVER_STOP_M;
    let mut closed = radius <= LOD_COVER_STOP_M || vertex_tight;
    // Below a few stops, one more split is cheaper than a coplanar-cover query,
    // and a mesh whose triangles are already that small never builds the grid.
    if !closed && radius > LOD_COVER_STOP_M * 4.0 {
        if let Some(planar) = planar_cover(onto, corners, index, radius, scratch) {
            // A loose slab is still a bound, but closing on it would miss the
            // 2-stop tightness the lattice checks. Keep splitting until the
            // certificate sits within one stop of the centroid distance.
            if planar <= upper && planar <= distance + LOD_COVER_STOP_M {
                upper = planar;
                closed = true;
            }
        }
    }
    upper.is_finite().then_some(Cover {
        corners,
        upper,
        depth,
        id,
        closed,
    })
}

/// `max` over the sub-triangle of distance to `tri` is attained at a vertex,
/// because that distance is convex. The surface is no farther than this one triangle.
fn vertex_certificate(tri: &MeshTriangle, corners: [[f32; 3]; 3]) -> Option<f32> {
    let mut worst = 0.0_f32;
    for corner in corners {
        let closest = tri.closest_point(corner);
        let span = distance(corner, closest);
        if !span.is_finite() {
            return None;
        }
        worst = worst.max(padded_distance(span, corner));
    }
    let worst = f32::next_up(worst);
    worst.is_finite().then_some(worst)
}

fn triangle_ref(tris: &[MeshTriangle], index: u32) -> Option<&MeshTriangle> {
    usize::try_from(index).ok().and_then(|slot| tris.get(slot))
}

/// Onto triangles a region may hand to the coplanar-cover test.
/// More than this and the region is still larger than the onto mesh, so it splits.
const PLANAR_CANDIDATES: usize = 128;
/// Cap on remainder polygons while subtracting. Past this the region splits
/// instead of letting the difference fan out.
const PLANAR_REMAINDER_CAP: usize = 512;

/// Upper bound from a coplanar cover.
///
/// Distance to one triangle is convex, so a vertex certificate is exact once a
/// single onto triangle contains the query. A flat sheet straddles many onto
/// triangles; those are coplanar, and distance to a plane is convex too. If the
/// onto triangles that meet the query lie in a slab of thickness `slab` and
/// subtracting their projections from the query polygon leaves no remainder,
/// every point of the query is within `plane distance + slab` of the surface
/// plus the offset mitre `eps / sin(a/2)`. A positive remainder is not a cover:
/// there is no area sum and no stand-in for the missing cap. That closes a
/// flat sheet at the onto mesh's triangle size instead of at [`LOD_COVER_STOP_M`].
fn planar_cover(
    onto: &OntoSurface<'_>,
    corners: [[f32; 3]; 3],
    plane_index: u32,
    radius: f32,
    scratch: &mut CoverScratch,
) -> Option<f32> {
    let plane_tri = triangle_ref(onto.tris, plane_index)?;
    let plane = Plane::from_triangle(plane_tri)?;
    let mut bounds_min = corners[0];
    let mut bounds_max = corners[0];
    for corner in corners {
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(corner[axis]);
            bounds_max[axis] = bounds_max[axis].max(corner[axis]);
        }
    }
    for axis in 0..3 {
        bounds_min[axis] -= LOD_COVER_STOP_M;
        bounds_max[axis] += LOD_COVER_STOP_M;
    }
    scratch.stamp = scratch.stamp.wrapping_add(1);
    if scratch.stamp == 0 {
        scratch.seen.fill(0);
        scratch.stamp = 1;
    }
    scratch.hits.clear();
    {
        let mut slot = onto.grid.borrow_mut();
        if slot.is_none() {
            *slot = Some(TriGrid::build(onto.tris));
        }
        let grid = slot.as_ref()?;
        grid.collect(
            bounds_min,
            bounds_max,
            &mut scratch.seen,
            scratch.stamp,
            &mut scratch.hits,
            PLANAR_CANDIDATES.saturating_add(1),
        );
    }
    if scratch.hits.is_empty() || scratch.hits.len() > PLANAR_CANDIDATES {
        return None;
    }
    scratch.hits.sort_unstable();
    let centre = centroid(corners);
    let slab = slab_of_nearby(
        onto.tris,
        &mut scratch.hits,
        &plane,
        centre,
        radius + LOD_COVER_STOP_M,
    )?;
    let mut query_height = 0.0_f32;
    for corner in corners {
        let height = plane.height(corner);
        if !height.is_finite() {
            return None;
        }
        query_height = query_height.max(height);
    }
    let query_2d = corners.map(|corner| plane.project(corner));
    if polygon_area(&query_2d) <= 0.0 {
        return None;
    }
    let eps = cover_eps(corners);
    // Growing each edge by `eps` mitres at a vertex: the spike reaches
    // `eps / sin(a/2)`, not `eps`. The close adds that reach.
    let reach = projections_cover(onto.tris, &scratch.hits, &plane, &query_2d, eps)?;
    let upper = f32::next_up(query_height + slab + reach);
    upper.is_finite().then_some(upper)
}

/// Slab thickness of the onto triangles that come within `reach` of `centre`.
/// Drops the rest. `None` when a nearby triangle leaves the slab, or none remain.
fn slab_of_nearby(
    tris: &[MeshTriangle],
    hits: &mut Vec<u32>,
    plane: &Plane,
    centre: [f32; 3],
    reach: f32,
) -> Option<f32> {
    let mut slab = 0.0_f32;
    let mut near = Vec::new();
    for index in hits.iter() {
        let tri = triangle_ref(tris, *index)?;
        let closest = tri.closest_point(centre);
        if distance(closest, centre) > reach {
            continue;
        }
        let mut local = plane.height(closest);
        for corner in tri.positions {
            if distance(corner, centre) <= reach {
                local = local.max(plane.height(corner));
            }
        }
        if !local.is_finite() || local > LOD_COVER_STOP_M {
            return None;
        }
        slab = slab.max(local);
        near.push(*index);
    }
    if near.is_empty() {
        return None;
    }
    hits.clone_from(&near);
    Some(slab)
}

struct Plane {
    normal: [f32; 3],
    offset: f32,
    tangent: [f32; 3],
    bitangent: [f32; 3],
}

impl Plane {
    fn from_triangle(tri: &MeshTriangle) -> Option<Self> {
        if tri.area() <= 0.0 {
            return None;
        }
        let normal = tri.face_normal();
        let offset = dot3(normal, tri.positions[0]);
        if !offset.is_finite() || normal.iter().any(|axis| !axis.is_finite()) {
            return None;
        }
        let helper = if normal[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let tangent = normalize3(cross3(normal, helper))?;
        let bitangent = cross3(normal, tangent);
        Some(Self {
            normal,
            offset,
            tangent,
            bitangent,
        })
    }

    fn height(&self, point: [f32; 3]) -> f32 {
        (dot3(self.normal, point) - self.offset).abs()
    }

    fn project(&self, point: [f32; 3]) -> [f32; 2] {
        [dot3(self.tangent, point), dot3(self.bitangent, point)]
    }
}

/// In-plane tolerance for the polygon difference. Larger than an `f32` ulp at
/// `scale`, smaller than a 0.1 mm step on the sheets this cook measures.
fn cover_eps(corners: [[f32; 3]; 3]) -> f32 {
    let mut scale = 1.0_f32;
    for corner in corners {
        for axis in corner {
            scale = scale.max(axis.abs());
        }
    }
    scale * 1.0e-6
}

/// Reach of an empty remainder, or `None` when area is left.
/// Each clip is removed once, so a duplicate or a partial overlap cannot fill a
/// hole. Clips are expanded by `eps` perpendicular to each edge; at a vertex of
/// interior angle `a` that mitre reaches `eps / sin(a/2)`. The returned reach
/// is the largest such spike among the clips that were subtracted.
fn projections_cover(
    tris: &[MeshTriangle],
    hits: &[u32],
    plane: &Plane,
    query: &[[f32; 2]; 3],
    eps: f32,
) -> Option<f32> {
    let mut remainders: Vec<Vec<[f32; 2]>> = vec![query.to_vec()];
    let mut reach = 0.0_f32;
    for index in hits {
        if remainders.is_empty() {
            break;
        }
        let tri = triangle_ref(tris, *index)?;
        let flat = tri.positions.map(|corner| plane.project(corner));
        if triangle_area_2d(flat) <= eps * eps {
            continue;
        }
        let mut next = Vec::new();
        let mut used = false;
        for polygon in &remainders {
            if !boxes_meet(polygon, flat, eps) {
                next.push(polygon.clone());
                continue;
            }
            used = true;
            next.extend(subtract_convex(polygon, flat, eps));
            if next.len() > PLANAR_REMAINDER_CAP {
                return None;
            }
        }
        if used {
            reach = reach.max(miter_reach(flat, eps)?);
        }
        remainders = next;
    }
    let clear = remainders
        .iter()
        .all(|polygon| polygon_area(polygon) <= 0.0);
    (clear && reach.is_finite() && reach > 0.0).then_some(reach)
}

/// Distance from a triangle vertex to the mitre of the two edges offset by `eps`.
/// That is `eps / sin(a/2)` for interior angle `a`. `None` when the angle
/// vanishes in `f32`, so the caller does not pretend the spike is bounded.
fn miter_reach(clip: [[f32; 2]; 3], eps: f32) -> Option<f32> {
    let mut reach = eps;
    for index in 0..3 {
        let here = clip[index];
        let prev = clip[(index + 2) % 3];
        let next = clip[(index + 1) % 3];
        let to_prev = [prev[0] - here[0], prev[1] - here[1]];
        let to_next = [next[0] - here[0], next[1] - here[1]];
        let len_prev = (to_prev[0] * to_prev[0] + to_prev[1] * to_prev[1]).sqrt();
        let len_next = (to_next[0] * to_next[0] + to_next[1] * to_next[1]).sqrt();
        if len_prev <= eps || len_next <= eps {
            return None;
        }
        let cos_angle = (to_prev[0] * to_next[0] + to_prev[1] * to_next[1]) / (len_prev * len_next);
        let cos_angle = cos_angle.clamp(-1.0, 1.0);
        let sin_half = f32::next_down(((1.0 - cos_angle) * 0.5).max(0.0).sqrt());
        if sin_half <= 0.0 || !sin_half.is_finite() {
            return None;
        }
        let spike = eps / sin_half;
        if !spike.is_finite() {
            return None;
        }
        reach = reach.max(spike);
    }
    Some(f32::next_up(reach))
}

fn boxes_meet(polygon: &[[f32; 2]], clip: [[f32; 2]; 3], eps: f32) -> bool {
    let Some(first) = polygon.first() else {
        return false;
    };
    let mut low = *first;
    let mut high = *first;
    for point in polygon {
        low[0] = low[0].min(point[0]);
        low[1] = low[1].min(point[1]);
        high[0] = high[0].max(point[0]);
        high[1] = high[1].max(point[1]);
    }
    let mut clip_low = clip[0];
    let mut clip_high = clip[0];
    for point in clip {
        clip_low[0] = clip_low[0].min(point[0]);
        clip_low[1] = clip_low[1].min(point[1]);
        clip_high[0] = clip_high[0].max(point[0]);
        clip_high[1] = clip_high[1].max(point[1]);
    }
    low[0] <= clip_high[0] + eps
        && high[0] >= clip_low[0] - eps
        && low[1] <= clip_high[1] + eps
        && high[1] >= clip_low[1] - eps
}

/// `subject` minus the edge-offset of a convex triangle (a mitre, not a disk).
/// The three outside pieces are disjoint: outside the first edge, then inside
/// that edge and outside the next, then inside both and outside the last.
fn subtract_convex(subject: &[[f32; 2]], clip: [[f32; 2]; 3], eps: f32) -> Vec<Vec<[f32; 2]>> {
    let mut inside = subject.to_vec();
    let mut outside = Vec::new();
    for edge in 0..3 {
        if inside.len() < 3 {
            break;
        }
        let start = clip[edge];
        let end = clip[(edge + 1) % 3];
        let third = clip[(edge + 2) % 3];
        let edge_len = span_len(start, end);
        if edge_len <= eps {
            return vec![subject.to_vec()];
        }
        let side = orient2(start, end, third);
        if side.abs() <= eps * edge_len {
            return vec![subject.to_vec()];
        }
        let (in_part, out_part) = split_halfplane(&inside, start, end, side > 0.0, eps, edge_len);
        if polygon_area(&out_part) > 0.0 {
            outside.push(out_part);
        }
        inside = in_part;
    }
    outside
}

fn split_halfplane(
    polygon: &[[f32; 2]],
    origin: [f32; 2],
    edge: [f32; 2],
    keep_positive: bool,
    eps: f32,
    edge_len: f32,
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    let mut inside = Vec::new();
    let mut outside = Vec::new();
    let Some(last) = polygon.last().copied() else {
        return (inside, outside);
    };
    let mut previous = last;
    let mut previous_in = in_expanded(origin, edge, previous, keep_positive, eps, edge_len);
    for point in polygon {
        let point_in = in_expanded(origin, edge, *point, keep_positive, eps, edge_len);
        if previous_in != point_in {
            if let Some(hit) =
                cross_offset(origin, edge, previous, *point, keep_positive, eps, edge_len)
            {
                inside.push(hit);
                outside.push(hit);
            }
        }
        if point_in {
            inside.push(*point);
        } else {
            outside.push(*point);
        }
        previous = *point;
        previous_in = point_in;
    }
    (inside, outside)
}

fn in_expanded(
    origin: [f32; 2],
    edge: [f32; 2],
    point: [f32; 2],
    keep_positive: bool,
    eps: f32,
    edge_len: f32,
) -> bool {
    let signed = orient2(origin, edge, point) / edge_len;
    if keep_positive {
        signed >= -eps
    } else {
        signed <= eps
    }
}

fn cross_offset(
    origin: [f32; 2],
    edge: [f32; 2],
    from: [f32; 2],
    to: [f32; 2],
    keep_positive: bool,
    eps: f32,
    edge_len: f32,
) -> Option<[f32; 2]> {
    let target = if keep_positive {
        -eps * edge_len
    } else {
        eps * edge_len
    };
    let from_side = orient2(origin, edge, from);
    let to_side = orient2(origin, edge, to);
    let denom = from_side - to_side;
    if !denom.is_finite() || denom.abs() <= f32::EPSILON {
        return Some([f32::midpoint(from[0], to[0]), f32::midpoint(from[1], to[1])]);
    }
    let weight = ((from_side - target) / denom).clamp(0.0, 1.0);
    let hit = [
        from[0] + (to[0] - from[0]) * weight,
        from[1] + (to[1] - from[1]) * weight,
    ];
    hit.iter().all(|axis| axis.is_finite()).then_some(hit)
}

fn span_len(start: [f32; 2], end: [f32; 2]) -> f32 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    (dx * dx + dy * dy).sqrt()
}

fn orient2(origin: [f32; 2], edge: [f32; 2], point: [f32; 2]) -> f32 {
    (edge[0] - origin[0]) * (point[1] - origin[1]) - (edge[1] - origin[1]) * (point[0] - origin[0])
}

fn polygon_area(polygon: &[[f32; 2]]) -> f32 {
    if polygon.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for (index, point) in polygon.iter().enumerate() {
        let next = polygon[(index + 1) % polygon.len()];
        sum += point[0] * next[1] - next[0] * point[1];
    }
    (sum * 0.5).abs()
}

fn triangle_area_2d(triangle: [[f32; 2]; 3]) -> f32 {
    polygon_area(&triangle)
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot3(value, value).sqrt();
    if length > 0.0 && length.is_finite() {
        Some([value[0] / length, value[1] / length, value[2] / length])
    } else {
        None
    }
}

struct TriGrid {
    origin: [f32; 3],
    cell: f32,
    dims: [usize; 3],
    offsets: Vec<u32>,
    indices: Vec<u32>,
}

impl TriGrid {
    fn build(tris: &[MeshTriangle]) -> Self {
        let mut bounds_min = [f32::INFINITY; 3];
        let mut bounds_max = [f32::NEG_INFINITY; 3];
        for tri in tris {
            for corner in tri.positions {
                for axis in 0..3 {
                    bounds_min[axis] = bounds_min[axis].min(corner[axis]);
                    bounds_max[axis] = bounds_max[axis].max(corner[axis]);
                }
            }
        }
        if tris.is_empty() || !bounds_min[0].is_finite() {
            return Self {
                origin: [0.0; 3],
                cell: 1.0,
                dims: [1, 1, 1],
                offsets: vec![0, 0],
                indices: Vec::new(),
            };
        }
        let span = [0, 1, 2].map(|axis| (bounds_max[axis] - bounds_min[axis]).max(0.0));
        let longest = span
            .into_iter()
            .fold(0.0_f32, f32::max)
            .max(LOD_COVER_STOP_M);
        let cell = (longest / 32.0).max(LOD_COVER_STOP_M);
        let dims = span.map(|axis| grid_dim(axis, cell));
        let bins = dims[0]
            .saturating_mul(dims[1])
            .saturating_mul(dims[2])
            .max(1);
        let mut counts = vec![0_u32; bins];
        for tri in tris {
            for slot in bin_slots(bounds_min, cell, dims, tri_bounds(tri)) {
                if let Some(count) = counts.get_mut(slot) {
                    *count = count.saturating_add(1);
                }
            }
        }
        let mut offsets = Vec::with_capacity(bins + 1);
        offsets.push(0_u32);
        for count in &counts {
            let last = offsets.last().copied().unwrap_or(0);
            offsets.push(last.saturating_add(*count));
        }
        let total = usize::try_from(offsets.last().copied().unwrap_or(0)).unwrap_or(0);
        let mut indices = vec![0_u32; total];
        let mut cursor = offsets.clone();
        for (index, tri) in tris.iter().enumerate() {
            let Ok(stored) = u32::try_from(index) else {
                continue;
            };
            for slot in bin_slots(bounds_min, cell, dims, tri_bounds(tri)) {
                let Some(at) = cursor.get_mut(slot) else {
                    continue;
                };
                if let Some(dest) = usize::try_from(*at)
                    .ok()
                    .and_then(|pos| indices.get_mut(pos))
                {
                    *dest = stored;
                    *at = at.saturating_add(1);
                }
            }
        }
        Self {
            origin: bounds_min,
            cell,
            dims,
            offsets,
            indices,
        }
    }

    fn collect(
        &self,
        bounds_min: [f32; 3],
        bounds_max: [f32; 3],
        seen: &mut [u32],
        stamp: u32,
        out: &mut Vec<u32>,
        limit: usize,
    ) {
        for slot in bin_slots(self.origin, self.cell, self.dims, (bounds_min, bounds_max)) {
            let Some(start) = self.offsets.get(slot).copied() else {
                continue;
            };
            let Some(end) = self.offsets.get(slot + 1).copied() else {
                continue;
            };
            let Ok(from) = usize::try_from(start) else {
                continue;
            };
            let Ok(to) = usize::try_from(end) else {
                continue;
            };
            for index in self.indices.get(from..to).unwrap_or(&[]) {
                let Ok(seen_at) = usize::try_from(*index) else {
                    continue;
                };
                if let Some(mark) = seen.get_mut(seen_at) {
                    if *mark != stamp {
                        *mark = stamp;
                        out.push(*index);
                        if out.len() >= limit {
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn tri_bounds(tri: &MeshTriangle) -> ([f32; 3], [f32; 3]) {
    let mut bounds_min = tri.positions[0];
    let mut bounds_max = tri.positions[0];
    for corner in tri.positions {
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(corner[axis]);
            bounds_max[axis] = bounds_max[axis].max(corner[axis]);
        }
    }
    (bounds_min, bounds_max)
}

fn grid_dim(span: f32, cell: f32) -> usize {
    if cell <= 0.0 {
        return 1;
    }
    lattice_slot(span / cell, 48).saturating_add(1).clamp(1, 48)
}

fn lattice_slot(value: f32, cap: usize) -> usize {
    if !(value.is_finite() && value > 0.0) || cap == 0 {
        return 0;
    }
    let mut slot = 0_usize;
    let mut left = value;
    let last = cap - 1;
    while left >= 1.0 && slot < last {
        left -= 1.0;
        slot += 1;
    }
    slot
}

fn bin_slots(
    origin: [f32; 3],
    cell: f32,
    dims: [usize; 3],
    bounds: ([f32; 3], [f32; 3]),
) -> Vec<usize> {
    if cell <= 0.0 {
        return Vec::new();
    }
    let lo = [0, 1, 2].map(|axis| lattice_slot((bounds.0[axis] - origin[axis]) / cell, dims[axis]));
    let hi = [0, 1, 2].map(|axis| lattice_slot((bounds.1[axis] - origin[axis]) / cell, dims[axis]));
    let mut slots = Vec::new();
    let mut z = lo[2];
    loop {
        let mut y = lo[1];
        loop {
            let mut x = lo[0];
            loop {
                slots.push(
                    (z.saturating_mul(dims[1]).saturating_add(y))
                        .saturating_mul(dims[0])
                        .saturating_add(x),
                );
                if x >= hi[0] {
                    break;
                }
                x += 1;
            }
            if y >= hi[1] {
                break;
            }
            y += 1;
        }
        if z >= hi[2] {
            break;
        }
        z += 1;
    }
    slots
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

/// Upper bound on the distance between any point of `from` and any point of `onto`.
fn pair_diagonal(from: &[[f32; 3]], onto: &[[f32; 3]]) -> f32 {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in from.iter().chain(onto.iter()) {
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
    let diagonal = (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt();
    if diagonal.is_finite() {
        f32::next_up(diagonal.max(1.0))
    } else {
        diagonal
    }
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
    use super::{
        combined_error, directed_bound, surface_bound, CoverBudget, LOD_COVER_STOP_M, LOD_EIGHTHS,
        LOD_RATIOS,
    };
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
        let mut budget = CoverBudget::full();
        assert!(surface_bound(&source, &level, &mut budget).is_none());
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
        let mut budget = CoverBudget::full();
        let bound = surface_bound(&flat_triangle(0.0), &flat_triangle(0.2), &mut budget)
            .unwrap_or(f32::NAN);
        assert!(
            bound >= 0.2,
            "lift of 0.2 m stored {bound}, which is under the displacement"
        );
        assert!(
            bound <= 0.2 + LOD_COVER_STOP_M,
            "lift of 0.2 m stored {bound}, looser than the stop radius"
        );
        assert!(!budget.limited);
    }

    #[test]
    fn a_spent_query_budget_still_bounds_the_surface() {
        let fine = grid_primitive(4, 1.0);
        let coarse = Primitive {
            positions: vec![[0.0, 0.0, 0.25], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..Primitive::default()
        };
        let mut budget = CoverBudget {
            left: 1,
            limited: false,
        };
        let stored = directed_bound(&coarse, &fine, &mut budget).unwrap_or(f32::NAN);
        assert!(budget.limited);
        assert!(stored.is_finite());
        let measured = lattice_against(&coarse, &fine, 8);
        assert!(
            measured <= stored,
            "lattice {measured} exceeds the budget-limited bound {stored}"
        );
    }

    fn grid_primitive(cells: usize, metres: f32) -> Primitive {
        let n = cells + 1;
        let mut positions = Vec::new();
        let span = f32::from(u16::try_from(cells).unwrap_or(1));
        for y in 0..=cells {
            for x in 0..=cells {
                let fx = f32::from(u16::try_from(x).unwrap_or(0)) / span;
                let fy = f32::from(u16::try_from(y).unwrap_or(0)) / span;
                positions.push([fx * metres, fy * metres, 0.0]);
            }
        }
        let mut indices = Vec::new();
        let row = u32::try_from(n).unwrap_or(0);
        for y in 0..cells {
            for x in 0..cells {
                let i = u32::try_from(y * n + x).unwrap_or(0);
                indices.extend([i, i + 1, i + row, i + 1, i + row + 1, i + row]);
            }
        }
        Primitive {
            positions,
            indices,
            ..Primitive::default()
        }
    }

    fn lattice_against(from: &Primitive, onto: &Primitive, segments: usize) -> f32 {
        let targets: Vec<genos_mesh::MeshTriangle> = onto
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|tri| {
                let corner = |slot: usize| {
                    tri.get(slot).and_then(|index| {
                        usize::try_from(*index)
                            .ok()
                            .and_then(|slot| onto.positions.get(slot).copied())
                    })
                };
                Some(genos_mesh::MeshTriangle::flat(
                    [corner(0)?, corner(1)?, corner(2)?],
                    0,
                    true,
                ))
            })
            .collect();
        let mut worst = 0.0_f32;
        let denom = f32::from(u16::try_from(segments).unwrap_or(1)).max(1.0);
        for tri in from.indices.as_chunks::<3>().0 {
            let corner = |slot: usize| {
                tri.get(slot).and_then(|index| {
                    usize::try_from(*index)
                        .ok()
                        .and_then(|slot| from.positions.get(slot).copied())
                })
            };
            let Some(corner_a) = corner(0) else { continue };
            let Some(corner_b) = corner(1) else { continue };
            let Some(corner_c) = corner(2) else { continue };
            for step_a in 0..=segments {
                let rest = segments - step_a;
                for step_b in 0..=rest {
                    let step_c = rest - step_b;
                    let weight = |step: usize, corner: [f32; 3], axis: usize| {
                        f32::from(u16::try_from(step).unwrap_or(0))
                            * corner.get(axis).copied().unwrap_or(0.0)
                    };
                    let point = [
                        (weight(step_a, corner_a, 0)
                            + weight(step_b, corner_b, 0)
                            + weight(step_c, corner_c, 0))
                            / denom,
                        (weight(step_a, corner_a, 1)
                            + weight(step_b, corner_b, 1)
                            + weight(step_c, corner_c, 1))
                            / denom,
                        (weight(step_a, corner_a, 2)
                            + weight(step_b, corner_b, 2)
                            + weight(step_c, corner_c, 2))
                            / denom,
                    ];
                    let mut best = f32::MAX;
                    for target in &targets {
                        let hit = target.closest_point(point);
                        let span = [point[0] - hit[0], point[1] - hit[1], point[2] - hit[2]];
                        let dist =
                            (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt();
                        best = best.min(dist);
                    }
                    worst = worst.max(best);
                }
            }
        }
        worst
    }

    fn assert_bound_covers_lattice(from: &Primitive, onto: &Primitive) {
        let mut budget = CoverBudget::full();
        let stored = surface_bound(from, onto, &mut budget).unwrap_or(f32::NAN);
        let measured = lattice_against(from, onto, 16).max(lattice_against(onto, from, 16));
        assert!(stored.is_finite(), "stored {stored}, lattice {measured}");
        assert!(
            measured <= stored + 1.0e-5,
            "lattice {measured} exceeds stored {stored}"
        );
    }

    fn holed_grid(duplicate: bool, overlap_half: bool) -> Primitive {
        let mut mesh = grid_primitive(4, 1.0);
        let row = 5_u32;
        let mut kept = Vec::new();
        for y in 0..4_u32 {
            for x in 0..4_u32 {
                if x == 1 && y == 1 {
                    continue;
                }
                let i = y * row + x;
                kept.extend([i, i + 1, i + row, i + 1, i + row + 1, i + row]);
            }
        }
        if duplicate {
            let copy = kept.clone();
            kept.extend(copy);
        }
        if overlap_half {
            let tris: Vec<[u32; 3]> = kept.as_chunks::<3>().0.to_vec();
            for tri in tris {
                let first = mesh
                    .positions
                    .get(usize::try_from(tri[0]).unwrap_or(0))
                    .copied();
                let third = mesh
                    .positions
                    .get(usize::try_from(tri[2]).unwrap_or(0))
                    .copied();
                let (Some(start), Some(end)) = (first, third) else {
                    continue;
                };
                let mid = [
                    f32::midpoint(start[0], end[0]),
                    f32::midpoint(start[1], end[1]),
                    f32::midpoint(start[2], end[2]),
                ];
                let Ok(index) = u32::try_from(mesh.positions.len()) else {
                    continue;
                };
                mesh.positions.push(mid);
                kept.extend([tri[0], tri[1], index]);
            }
        }
        mesh.indices = kept;
        mesh
    }

    #[test]
    fn duplicated_triangles_do_not_fill_a_hole() {
        let onto = holed_grid(true, false);
        let from = grid_primitive(1, 1.0);
        let mut budget = CoverBudget::full();
        let stored = surface_bound(&from, &onto, &mut budget).unwrap_or(f32::NAN);
        let measured = lattice_against(&from, &onto, 16);
        assert!(
            measured >= 0.05,
            "lattice {measured} did not see the 0.25 cell hole"
        );
        assert!(
            measured <= stored + 1.0e-5,
            "duplicated cover stored {stored}, lattice {measured}"
        );
    }

    #[test]
    fn overlapping_triangles_do_not_fill_a_hole() {
        let onto = holed_grid(false, true);
        let from = grid_primitive(1, 1.0);
        let mut budget = CoverBudget::full();
        let stored = surface_bound(&from, &onto, &mut budget).unwrap_or(f32::NAN);
        let measured = lattice_against(&from, &onto, 16);
        assert!(measured >= 0.05, "lattice {measured} did not see the hole");
        assert!(
            measured <= stored + 1.0e-5,
            "overlapping cover stored {stored}, lattice {measured}"
        );
    }

    #[test]
    fn a_five_millimetre_corner_cap_is_not_under_reported() {
        // Equilateral, altitude 0.52 m. The 5 mm cap is a similar triangle, so
        // its area fraction is under 1e-4 and `radius * sqrt(deficit)` is about
        // 3.3 mm, inside the one-stop gate.
        let height = 0.52_f32;
        let half = height / 3.0_f32.sqrt();
        let base_x = -0.005 + height;
        let cut_half = half * (0.005 / height);
        let from = Primitive {
            positions: vec![
                [-0.005, 0.0, 0.0],
                [base_x, -half, 0.0],
                [base_x, half, 0.0],
            ],
            indices: vec![0, 1, 2],
            ..Primitive::default()
        };
        let onto = Primitive {
            positions: vec![
                [0.0, -cut_half, 0.0],
                [base_x, -half, 0.0],
                [base_x, half, 0.0],
                [0.0, cut_half, 0.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Primitive::default()
        };
        let mut budget = CoverBudget::full();
        let stored = surface_bound(&from, &onto, &mut budget).unwrap_or(f32::NAN);
        let measured = lattice_against(&from, &onto, 16);
        assert!(measured >= 0.004, "lattice {measured} missed the 5 mm cap");
        assert!(
            measured <= stored + 1.0e-5,
            "cap stored {stored}, lattice {measured}"
        );
    }

    #[test]
    fn a_tenth_millimetre_step_stays_above_the_lattice() {
        let onto = Primitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [0.5, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.5, 1.0, 0.0],
                [0.5, 0.0, 0.0001],
                [1.0, 0.0, 0.0001],
                [0.5, 1.0, 0.0001],
                [1.0, 1.0, 0.0001],
            ],
            indices: vec![0, 1, 2, 1, 3, 2, 4, 5, 6, 5, 7, 6],
            ..Primitive::default()
        };
        let from = Primitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2, 1, 3, 2],
            ..Primitive::default()
        };
        assert_bound_covers_lattice(&from, &onto);
        let measured = lattice_against(&from, &onto, 16);
        assert!(
            measured >= 5.0e-5,
            "lattice {measured} missed the 0.1 mm step"
        );
    }

    #[test]
    fn a_half_degree_crease_stays_above_the_lattice() {
        let rise = 0.5_f32.to_radians().tan();
        let onto = Primitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [0.5, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.5, 1.0, 0.0],
                [1.0, 0.0, rise],
                [1.0, 1.0, rise],
            ],
            indices: vec![0, 1, 2, 1, 3, 2, 1, 4, 3, 4, 5, 3],
            ..Primitive::default()
        };
        let from = Primitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2, 1, 3, 2],
            ..Primitive::default()
        };
        assert_bound_covers_lattice(&from, &onto);
        let measured = lattice_against(&from, &onto, 16);
        assert!(
            measured >= 0.5 * rise * 0.5,
            "lattice {measured} missed the crease of {rise}"
        );
    }

    /// Query is the onto triangle with one base vertex pulled `overhang` metres
    /// past the sharp corner. Analytic truth is that overhang.
    fn assert_sharp_overhang(origin_x: f32, height: f32, overhang: f32) {
        let left = [origin_x, 0.0, 0.0];
        let right = [origin_x + 1.0, 0.0, 0.0];
        let apex = [origin_x + 0.5, height, 0.0];
        let hung = [origin_x - overhang, 0.0, 0.0];
        let onto = Primitive {
            positions: vec![left, right, apex],
            indices: vec![0, 1, 2],
            ..Primitive::default()
        };
        let from = Primitive {
            positions: vec![hung, right, apex],
            indices: vec![0, 1, 2],
            ..Primitive::default()
        };
        let mut budget = CoverBudget::full();
        let stored = surface_bound(&from, &onto, &mut budget).unwrap_or(f32::NAN);
        let measured = lattice_against(&from, &onto, 8);
        assert!(
            stored + 1.0e-3 >= overhang,
            "stored {stored} under the {overhang} m overhang (height {height}, x {origin_x})"
        );
        assert!(
            measured <= stored + 1.0e-3,
            "lattice {measured} exceeds stored {stored}"
        );
    }

    #[test]
    fn a_five_micrometre_spike_does_not_cover_five_centimetres() {
        assert_sharp_overhang(0.0, 5.0e-6, 0.05);
    }

    #[test]
    fn a_five_centimetre_spike_at_long_range_does_not_cover_two_centimetres() {
        assert_sharp_overhang(5000.0, 0.05, 0.02);
    }

    #[test]
    fn a_five_centimetre_spike_at_long_range_does_not_cover_five_centimetres() {
        assert_sharp_overhang(5000.0, 0.05, 0.05);
    }

    #[test]
    fn a_one_centimetre_spike_at_long_range_does_not_cover_ten_centimetres() {
        assert_sharp_overhang(5000.0, 0.01, 0.10);
    }
}
