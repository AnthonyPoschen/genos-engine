//! The `.gbake` file: little-endian, versioned. A file of another version is
//! ignored (and rebuilt in on-load mode).

use crate::cards::{Card, CardTexel, Cards};
use crate::sdf::{MeshSdf, SdfBrick, BRICK};
use crate::Baked;

const MAGIC: &[u8; 4] = b"GBK1";
pub const VERSION: u32 = 1;

pub fn encode(b: &Baked) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend_from_slice(MAGIC);
    u32_(&mut w, VERSION);
    w.extend_from_slice(&b.key.to_le_bytes());
    let s = &b.sdf;
    s.origin.iter().for_each(|&v| f32_(&mut w, v));
    f32_(&mut w, s.voxel);
    s.bricks_dims.iter().for_each(|&v| u32_(&mut w, v));
    f32_(&mut w, s.band);
    f32_(&mut w, s.coarse_band);
    w.push(s.thin as u8);
    u32_(&mut w, s.coarse.len() as u32);
    w.extend_from_slice(&s.coarse);
    u32_(&mut w, s.bricks.len() as u32);
    for brick in &s.bricks {
        brick.at.iter().for_each(|&v| u32_(&mut w, v));
        w.extend_from_slice(&brick.distances);
    }
    u32_(&mut w, b.cards.cards.len() as u32);
    for c in &b.cards.cards {
        w.push(c.face);
        c.origin.iter().for_each(|&v| f32_(&mut w, v));
        c.texel.iter().for_each(|&v| f32_(&mut w, v));
        u32_(&mut w, c.width);
        u32_(&mut w, c.height);
        f32_(&mut w, c.depth);
        for t in &c.texels {
            w.extend_from_slice(&t.albedo);
            w.extend_from_slice(&t.normal);
            w.extend_from_slice(&t.depth.to_le_bytes());
            t.emissive.iter().for_each(|&v| f32_(&mut w, v));
        }
    }
    w
}

pub fn decode(bytes: &[u8]) -> Option<Baked> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let key = u64::from_le_bytes(r.take(8)?.try_into().ok()?);
    let origin = [r.f32()?, r.f32()?, r.f32()?];
    let voxel = r.f32()?;
    let bricks_dims = [r.u32()?, r.u32()?, r.u32()?];
    let band = r.f32()?;
    let coarse_band = r.f32()?;
    let thin = r.take(1)?[0] != 0;
    let n = r.u32()? as usize;
    let coarse = r.take(n)?.to_vec();
    let n = r.u32()? as usize;
    let mut bricks = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let at = [r.u32()?, r.u32()?, r.u32()?];
        let distances = r.take(BRICK * BRICK * BRICK)?.to_vec();
        bricks.push(SdfBrick { at, distances });
    }
    let n = r.u32()? as usize;
    let mut cards = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        let face = r.take(1)?[0];
        let origin = [r.f32()?, r.f32()?, r.f32()?];
        let texel = [r.f32()?, r.f32()?];
        let (width, height) = (r.u32()?, r.u32()?);
        let depth = r.f32()?;
        let count = (width as usize).checked_mul(height as usize)?;
        let mut texels = Vec::with_capacity(count.min(1 << 24));
        for _ in 0..count {
            let a = r.take(4)?;
            let nrm = r.take(3)?;
            let d = r.take(2)?;
            texels.push(CardTexel {
                albedo: [a[0], a[1], a[2], a[3]],
                normal: [nrm[0], nrm[1], nrm[2]],
                depth: u16::from_le_bytes([d[0], d[1]]),
                emissive: [r.f32()?, r.f32()?, r.f32()?],
            });
        }
        cards.push(Card {
            face,
            origin,
            texel,
            width,
            height,
            depth,
            texels,
        });
    }
    (r.at == bytes.len()).then_some(Baked {
        key,
        sdf: MeshSdf {
            origin,
            voxel,
            bricks_dims,
            band,
            bricks,
            coarse,
            coarse_band,
            thin,
        },
        cards: Cards { cards },
    })
}

fn u32_(w: &mut Vec<u8>, v: u32) {
    w.extend_from_slice(&v.to_le_bytes());
}
fn f32_(w: &mut Vec<u8>, v: f32) {
    w.extend_from_slice(&v.to_le_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}
