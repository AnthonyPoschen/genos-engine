//! Surface cache cards of one mesh: six axis-aligned views of its bounds.
//!
//! Each card texel looks into the mesh along the card's axis and records the first
//! surface that faces it: diffuse albedo, normal, depth, and emission. Four
//! sub-samples per texel: albedo and normal average over the hits, coverage is
//! the hit share, and emission averages over all four (its area-weighted mean, so
//! a small bright detail keeps its power). The texel size is fixed per mesh by the
//! settings, never by camera distance. A complex mesh will get more cards from a
//! greedy cover (GI v2 phase 4); six is the simple-mesh case.

use genos_mesh::Bvh;

use crate::sdf::parallel_map;
use crate::{BakeMesh, BakeSettings};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CardTexel {
    /// Linear diffuse albedo x 255, and coverage x 255 (0: nothing here).
    pub albedo: [u8; 4],
    /// Normal, each component (n + 1) / 2 x 255.
    pub normal: [u8; 3],
    /// Distance from the card plane into the bounds, as a fraction of the depth x 65535.
    pub depth: u16,
    /// Emitted radiance (linear).
    pub emissive: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// 0..6: +x, -x, +y, -y, +z, -z is the direction the card looks from (its normal).
    pub face: u8,
    /// Card rectangle corner on the bounds face (object space).
    pub origin: [f32; 3],
    /// Texel edge in metres along the card's two axes.
    pub texel: [f32; 2],
    pub width: u32,
    pub height: u32,
    /// Bounds depth along the axis.
    pub depth: f32,
    pub texels: Vec<CardTexel>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Cards {
    pub cards: Vec<Card>,
}

impl Cards {
    pub fn texel_count(&self) -> usize {
        self.cards.iter().map(|c| c.texels.len()).sum()
    }

    /// Total emitted power seen by the cards: sum of emission x texel area, per face.
    pub fn emitted(&self, face: u8) -> [f32; 3] {
        let mut sum = [0.0; 3];
        for c in self.cards.iter().filter(|c| c.face == face) {
            let area = c.texel[0] * c.texel[1];
            for t in &c.texels {
                for (s, e) in sum.iter_mut().zip(t.emissive) {
                    *s += e * area;
                }
            }
        }
        sum
    }
}

/// (axis, sign) of each face and the two in-plane axes.
const FACES: [(usize, f32, usize, usize); 6] = [
    (0, 1.0, 2, 1),
    (0, -1.0, 2, 1),
    (1, 1.0, 0, 2),
    (1, -1.0, 0, 2),
    (2, 1.0, 0, 1),
    (2, -1.0, 0, 1),
];

pub fn build(mesh: &BakeMesh, s: &BakeSettings) -> Cards {
    if mesh.triangles.is_empty() {
        return Cards::default();
    }
    let tris = &mesh.triangles;
    let bvh = Bvh::build(tris);
    let b = mesh.bounds();
    let mut cards = Vec::new();
    for (face, &(axis, sign, u, v)) in FACES.iter().enumerate() {
        let size = |a: usize| (b.max[a] - b.min[a]).max(1.0e-4);
        let count = |a: usize| ((size(a) / s.card_texel).ceil() as u32).clamp(1, s.card_max_texels);
        let (width, height) = (count(u), count(v));
        let texel = [size(u) / width as f32, size(v) / height as f32];
        let depth = size(axis);
        // Start rays a little outside the bounds (beyond the BVH's minimum hit distance).
        let pad = 0.01 + 1.0e-3 * depth;
        let mut origin = b.min;
        origin[axis] = if sign > 0.0 { b.max[axis] } else { b.min[axis] };
        let mut dir = [0.0; 3];
        dir[axis] = -sign;
        let pixels: Vec<u32> = (0..width * height).collect();
        let texels = parallel_map(&pixels, |&i| {
            let (x, y) = (i % width, i / width);
            let mut out = CardTexel::default();
            let (mut alb, mut nrm, mut dep, mut emi, mut hits) =
                ([0.0f32; 3], [0.0f32; 3], 0.0f32, [0.0f32; 3], 0u32);
            for sub in 0..4 {
                let (fx, fy) = (0.25 + 0.5 * (sub % 2) as f32, 0.25 + 0.5 * (sub / 2) as f32);
                let mut o = origin;
                o[u] += (x as f32 + fx) * texel[0];
                o[v] += (y as f32 + fy) * texel[1];
                o[axis] += sign * pad;
                let Some(hit) = bvh.intersect(tris, o, dir, depth + 2.0 * pad) else {
                    continue;
                };
                let t = &tris[hit.triangle as usize];
                let (a, e) = mesh.surface(t.material, t.uv_at(hit.u, hit.v));
                let mut n = t.normal_at(hit.u, hit.v);
                if hit.back {
                    n = n.map(|c| -c);
                }
                for c in 0..3 {
                    alb[c] += a[c];
                    nrm[c] += n[c];
                    emi[c] += e[c];
                }
                dep += (hit.t - pad).max(0.0);
                hits += 1;
            }
            if hits > 0 {
                let k = 1.0 / hits as f32;
                let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2])
                    .sqrt()
                    .max(1.0e-6);
                out.albedo = [
                    q8(alb[0] * k),
                    q8(alb[1] * k),
                    q8(alb[2] * k),
                    q8(hits as f32 / 4.0),
                ];
                out.normal = nrm.map(|c| q8(c / len * 0.5 + 0.5));
                out.depth = ((dep * k / depth).clamp(0.0, 1.0) * 65535.0).round() as u16;
                out.emissive = emi.map(|c| c / 4.0);
            }
            out
        });
        cards.push(Card {
            face: face as u8,
            origin,
            texel,
            width,
            height,
            depth,
            texels,
        });
    }
    Cards { cards }
}

fn q8(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}
