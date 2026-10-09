//! Mesh SDF instances for the GPU software tracer (GI v2 phase 2).
//!
//! A traced mesh carries its baked signed distance field (genos-bake); every placed
//! instance of it is one [`FieldInstance`] in `World::fields`. Each frame the
//! instances and the shared SDFs pack into one word buffer that `mesh_field.glsl`
//! sphere-traces behind the tracer interface (`trace_ray` / `trace_occluded`).
//!
//! Word layout (all `u32`, floats as bits):
//!
//! ```text
//! [0]  instance count   [1] max march steps   [2] reserved   [3] reserved
//! per instance, 32 words from word 4:
//!   0..12  world -> object rows (3 x vec4: m0 m1 m2 t)
//!   12..15 world bounds low, 15 distance scale (object metres -> world metres)
//!   16..19 world bounds high, 19 albedo (unpackUnorm4x8; a = 1 when two-sided)
//!   20..23 grid origin (object), 23 voxel
//!   24..27 grid size in bricks (x y z), 27 brick table word
//!   28 band, 29 coarse band, 30 coarse word, 31 thin (1) or closed (0)
//! brick table: one word per brick cell, 0 for none, else 1 + the brick's first word
//! brick: 128 words, 512 distances as bytes, x fastest
//! coarse: one byte per brick cell, four per word
//! ```

use std::sync::Arc;

use genos_bake::{MeshSdf, BRICK};

/// Words per instance record.
pub const FIELD_RECORD: usize = 32;
/// Header words before the first record.
pub const FIELD_HEADER: usize = 4;
/// Most sphere-trace steps one instance takes per ray.
pub const FIELD_STEPS: u32 = 96;

/// One placed, traced mesh.
#[derive(Clone, Debug)]
pub struct FieldInstance {
    pub sdf: Arc<MeshSdf>,
    /// Object -> world, column-major.
    pub pose: [f32; 16],
    /// Mean diffuse albedo, linear.
    pub albedo: [f32; 3],
    /// The world object whose `hidden` flag this instance follows, if any.
    pub object: Option<usize>,
}

impl FieldInstance {
    /// World-space bounds of the SDF grid.
    pub fn world_bounds(&self) -> ([f32; 3], [f32; 3]) {
        let s = &self.sdf;
        let size = s.voxel * BRICK as f32;
        let hi: [f32; 3] = std::array::from_fn(|a| s.origin[a] + s.bricks_dims[a] as f32 * size);
        let (mut lo_w, mut hi_w) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for k in 0..8 {
            let c = [
                if k & 1 == 0 { s.origin[0] } else { hi[0] },
                if k & 2 == 0 { s.origin[1] } else { hi[1] },
                if k & 4 == 0 { s.origin[2] } else { hi[2] },
            ];
            let w = crate::world::transform_pose(&self.pose, c);
            for a in 0..3 {
                lo_w[a] = lo_w[a].min(w[a]);
                hi_w[a] = hi_w[a].max(w[a]);
            }
        }
        (lo_w, hi_w)
    }
}

/// Inverse of an affine column-major pose, as three rows of (m, t).
pub fn inverse_rows(pose: &[f32; 16]) -> Option<[[f32; 4]; 3]> {
    let m = |r: usize, c: usize| pose[c * 4 + r];
    let det = m(0, 0) * (m(1, 1) * m(2, 2) - m(1, 2) * m(2, 1))
        - m(0, 1) * (m(1, 0) * m(2, 2) - m(1, 2) * m(2, 0))
        + m(0, 2) * (m(1, 0) * m(2, 1) - m(1, 1) * m(2, 0));
    if det.abs() < 1.0e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let mut r = [[0.0f32; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            // cofactor of (j, i)
            let (a0, a1) = ((j + 1) % 3, (j + 2) % 3);
            let (b0, b1) = ((i + 1) % 3, (i + 2) % 3);
            r[i][j] = (m(a0, b0) * m(a1, b1) - m(a0, b1) * m(a1, b0)) * inv;
        }
    }
    for row in &mut r {
        row[3] = -(row[0] * m(0, 3) + row[1] * m(1, 3) + row[2] * m(2, 3));
    }
    Some(r)
}

/// World metres per object metre, taking the smallest stretch so a world step never
/// overshoots: 1 / the largest row norm of the inverse.
fn distance_scale(rows: &[[f32; 4]; 3]) -> f32 {
    // The inverse's largest singular value is at most its Frobenius-like row bound;
    // for uniform scale and rotation this is exact.
    let mut most = 0.0f32;
    for c in 0..3 {
        let n = (0..3).map(|r| rows[r][c] * rows[r][c]).sum::<f32>().sqrt();
        most = most.max(n);
    }
    for row in rows {
        let n = (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt();
        most = most.max(n);
    }
    if most > 0.0 {
        1.0 / most
    } else {
        1.0
    }
}

/// The instances that trace this frame.
pub fn live_instances(world: &crate::world::World) -> Vec<&FieldInstance> {
    world
        .fields
        .iter()
        .filter(|f| {
            f.object
                .and_then(|o| world.objects.get(o))
                .is_none_or(|o| !o.hidden)
        })
        .collect()
}

/// Pack the instances (and the SDFs they use, once each) into the word buffer.
pub fn field_words(instances: &[&FieldInstance]) -> Vec<u32> {
    let f = |v: f32| v.to_bits();
    let mut words = vec![0u32; FIELD_HEADER];
    let mut records: Vec<[u32; FIELD_RECORD]> = Vec::new();
    // Shared SDFs: (pointer, table word, coarse word)
    let mut placed: Vec<(*const MeshSdf, u32, u32)> = Vec::new();
    let mut data: Vec<u32> = Vec::new();
    let mut kept: Vec<(&FieldInstance, [[f32; 4]; 3])> = Vec::new();
    for inst in instances {
        if let Some(rows) = inverse_rows(&inst.pose) {
            kept.push((inst, rows));
        }
    }
    let data_at = FIELD_HEADER + kept.len() * FIELD_RECORD;
    for (inst, rows) in &kept {
        let ptr = Arc::as_ptr(&inst.sdf);
        let (table, coarse) = match placed.iter().find(|p| p.0 == ptr) {
            Some(p) => (p.1, p.2),
            None => {
                let s = &inst.sdf;
                let cells = (s.bricks_dims[0] * s.bricks_dims[1] * s.bricks_dims[2]) as usize;
                let table_at = (data_at + data.len()) as u32;
                let table_start = data.len();
                data.resize(data.len() + cells, 0);
                let coarse_at = (data_at + data.len()) as u32;
                for chunk in s.coarse.chunks(4) {
                    let mut w = 0u32;
                    for (k, b) in chunk.iter().enumerate() {
                        w |= (*b as u32) << (8 * k);
                    }
                    data.push(w);
                }
                for brick in &s.bricks {
                    let d = s.bricks_dims;
                    let cell = ((brick.at[2] * d[1] + brick.at[1]) * d[0] + brick.at[0]) as usize;
                    data[table_start + cell] = (data_at + data.len()) as u32 + 1;
                    for chunk in brick.distances.chunks(4) {
                        data.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                    }
                }
                placed.push((ptr, table_at, coarse_at));
                (table_at, coarse_at)
            }
        };
        let s = &inst.sdf;
        let (lo, hi) = inst.world_bounds();
        let mut r = [0u32; FIELD_RECORD];
        for (i, row) in rows.iter().enumerate() {
            for c in 0..4 {
                r[i * 4 + c] = f(row[c]);
            }
        }
        r[12..15].copy_from_slice(&lo.map(f));
        r[15] = f(distance_scale(rows));
        r[16..19].copy_from_slice(&hi.map(f));
        let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        r[19] = unorm(inst.albedo[0])
            | unorm(inst.albedo[1]) << 8
            | unorm(inst.albedo[2]) << 16
            | if s.thin { 255 << 24 } else { 0 };
        r[20..23].copy_from_slice(&s.origin.map(f));
        r[23] = f(s.voxel);
        r[24..27].copy_from_slice(&s.bricks_dims);
        r[27] = table;
        r[28] = f(s.band);
        r[29] = f(s.coarse_band);
        r[30] = coarse;
        r[31] = u32::from(s.thin);
        records.push(r);
    }
    words[0] = records.len() as u32;
    words[1] = FIELD_STEPS;
    for r in &records {
        words.extend_from_slice(r);
    }
    words.extend_from_slice(&data);
    words
}

/// Byte view of [`field_words`].
pub fn field_bytes(instances: &[&FieldInstance]) -> Vec<u8> {
    field_words(instances)
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}

/// A CPU twin of the shader's distance lookup, for tests: world-space distance to
/// the instance's surface at `p` (trilinear over voxel centres, missing bricks at
/// the band).
pub fn instance_distance(inst: &FieldInstance, p: [f32; 3]) -> Option<f32> {
    let rows = inverse_rows(&inst.pose)?;
    let o: [f32; 3] = std::array::from_fn(|i| {
        rows[i][0] * p[0] + rows[i][1] * p[1] + rows[i][2] * p[2] + rows[i][3]
    });
    Some(object_distance(&inst.sdf, o) * distance_scale(&rows))
}

fn object_distance(s: &MeshSdf, p: [f32; 3]) -> f32 {
    let dims: [i32; 3] = std::array::from_fn(|a| (s.bricks_dims[a] as usize * BRICK) as i32);
    let g: [f32; 3] = std::array::from_fn(|a| (p[a] - s.origin[a]) / s.voxel - 0.5);
    let v0: [i32; 3] = std::array::from_fn(|a| g[a].floor() as i32);
    let fr: [f32; 3] = std::array::from_fn(|a| g[a] - v0[a] as f32);
    let voxel = |v: [i32; 3]| -> f32 {
        if (0..3).any(|a| v[a] < 0 || v[a] >= dims[a]) {
            return s.band;
        }
        let b: [u32; 3] = std::array::from_fn(|a| (v[a] as usize / BRICK) as u32);
        match s.bricks.iter().find(|k| k.at == b) {
            Some(brick) => {
                let l: [usize; 3] = std::array::from_fn(|a| v[a] as usize % BRICK);
                genos_bake::sdf_decode(
                    brick.distances[(l[2] * BRICK + l[1]) * BRICK + l[0]],
                    s.band,
                )
            }
            None => s.band,
        }
    };
    let mut d = 0.0;
    for k in 0..8 {
        let o = [k & 1, (k >> 1) & 1, (k >> 2) & 1];
        let w: f32 = (0..3)
            .map(|a| if o[a] == 1 { fr[a] } else { 1.0 - fr[a] })
            .product();
        d += w * voxel(std::array::from_fn(|a| v0[a] + o[a] as i32));
    }
    d
}

/// Changes when the traced instances change (added, moved, hidden). Never 0, so a
/// fresh GPU (key 0) writes the empty set once.
pub fn field_key(instances: &[&FieldInstance]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    eat(instances.len() as u64);
    for inst in instances {
        eat(Arc::as_ptr(&inst.sdf) as usize as u64);
        for v in inst.pose {
            eat(v.to_bits() as u64);
        }
        for v in inst.albedo {
            eat(v.to_bits() as u64);
        }
    }
    h | 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_bake::{bake, BakeMesh, BakeMode, BakeSettings};
    use genos_mesh::MeshTriangle;

    fn cube(h: f32) -> Vec<MeshTriangle> {
        let c = |i: usize| [0, 1, 2].map(|a| if i >> a & 1 == 1 { h } else { -h });
        let faces = [
            [1, 3, 7, 5],
            [0, 4, 6, 2],
            [2, 6, 7, 3],
            [0, 1, 5, 4],
            [4, 5, 7, 6],
            [0, 2, 3, 1],
        ];
        faces
            .iter()
            .flat_map(|f| {
                [
                    MeshTriangle::flat([c(f[0]), c(f[1]), c(f[2])], 0, false),
                    MeshTriangle::flat([c(f[0]), c(f[2]), c(f[3])], 0, false),
                ]
            })
            .collect()
    }

    fn cube_instance(pose: [f32; 16]) -> FieldInstance {
        let settings = BakeSettings {
            mode: BakeMode::OnLoad,
            cache_dir: std::env::temp_dir().join("genos-mesh-field-test"),
            ..BakeSettings::default()
        };
        let mesh = BakeMesh::plain("cube", cube(0.5), &[[0.5; 3]]);
        FieldInstance {
            sdf: Arc::new(bake(&mesh, &settings).sdf),
            pose,
            albedo: [0.5, 0.25, 1.0],
            object: None,
        }
    }

    #[test]
    fn a_scaled_moved_cube_reads_back_its_world_distance() {
        // Scale 2 and move to (10, 1, -3): a 2 m cube centred there.
        let mut pose = crate::world::identity_pose();
        pose[0] = 2.0;
        pose[5] = 2.0;
        pose[10] = 2.0;
        pose[12] = 10.0;
        pose[13] = 1.0;
        pose[14] = -3.0;
        let inst = cube_instance(pose);
        let voxel = inst.sdf.voxel * 2.0;
        for (p, want) in [
            ([11.1, 1.2, -3.3], 0.1),
            ([10.9, 1.0, -3.0], -0.1),
            ([10.0, 2.05, -2.5], 0.05),
            ([10.3, 0.2, -3.95], -0.05),
        ] {
            let d = instance_distance(&inst, p).unwrap();
            assert!((d - want).abs() < voxel, "{p:?}: {d} vs {want}");
        }
        let (lo, hi) = inst.world_bounds();
        assert!(lo[0] < 9.0 && hi[0] > 11.0 && lo[1] < 0.0 && hi[1] > 2.0);
    }

    #[test]
    fn instances_share_one_copy_of_their_sdf() {
        let a = cube_instance(crate::world::identity_pose());
        let mut b = a.clone();
        b.pose[12] = 5.0;
        let one = field_words(&[&a]);
        let two = field_words(&[&a, &b]);
        assert_eq!(one[0], 1);
        assert_eq!(two[0], 2);
        // The second instance costs a record, not a second SDF.
        assert_eq!(two.len() - one.len(), FIELD_RECORD);
        // Every stored brick has a table entry pointing past the records.
        let base = FIELD_HEADER;
        let table = two[base + 27] as usize;
        let d = a.sdf.bricks_dims;
        let cells = (d[0] * d[1] * d[2]) as usize;
        let stored = two[table..table + cells]
            .iter()
            .filter(|w| **w != 0)
            .count();
        assert_eq!(stored, a.sdf.bricks.len());
        assert_eq!(
            two[base + 27],
            two[base + FIELD_RECORD + 27],
            "shared table"
        );
        assert_eq!(two[base + 19] & 0xff, 128, "albedo red 0.5");
        assert_ne!(field_key(&[&a]), field_key(&[&a, &b]));
    }

    #[test]
    fn inverse_rows_undo_the_pose() {
        let mut pose = crate::world::identity_pose();
        // 90 degrees about Y, scale 3, moved.
        pose[0] = 0.0;
        pose[2] = -3.0;
        pose[8] = 3.0;
        pose[10] = 0.0;
        pose[5] = 3.0;
        pose[12] = 1.0;
        pose[13] = 2.0;
        pose[14] = 3.0;
        let rows = inverse_rows(&pose).unwrap();
        let p = [0.3, -0.2, 0.7];
        let w = crate::world::transform_pose(&pose, p);
        let back: [f32; 3] = std::array::from_fn(|i| {
            rows[i][0] * w[0] + rows[i][1] * w[1] + rows[i][2] * w[2] + rows[i][3]
        });
        for a in 0..3 {
            assert!((back[a] - p[a]).abs() < 1e-5, "{back:?}");
        }
        assert!((distance_scale(&rows) - 3.0).abs() < 1e-5);
    }
}
