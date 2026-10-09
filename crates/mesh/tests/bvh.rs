//! The BVH must agree with brute force on every query.

use genos_mesh::{Bvh, MeshTriangle};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn point(&mut self, s: f32) -> [f32; 3] {
        [
            (self.next() - 0.5) * s,
            (self.next() - 0.5) * s,
            (self.next() - 0.5) * s,
        ]
    }
}

fn soup(n: usize, rng: &mut Rng) -> Vec<MeshTriangle> {
    (0..n)
        .map(|i| {
            let c = rng.point(20.0);
            let p = [0, 1, 2].map(|_| {
                let d = rng.point(1.5);
                [c[0] + d[0], c[1] + d[1], c[2] + d[2]]
            });
            MeshTriangle::flat(p, i as u32, i % 3 == 0)
        })
        .collect()
}

fn brute(tris: &[MeshTriangle], o: [f32; 3], d: [f32; 3], t_max: f32) -> Option<(u32, f32)> {
    let mut best: Option<(u32, f32)> = None;
    for (i, t) in tris.iter().enumerate() {
        let single = Bvh::build(std::slice::from_ref(t));
        if let Some(h) =
            single.intersect(std::slice::from_ref(t), o, d, best.map_or(t_max, |b| b.1))
        {
            best = Some((i as u32, h.t));
        }
    }
    best
}

#[test]
fn rays_match_brute_force() {
    let mut rng = Rng(0x1234_5678_9abc);
    let tris = soup(2000, &mut rng);
    let bvh = Bvh::build(&tris);
    assert!(bvh.node_count() > 100);
    let mut hits = 0;
    for _ in 0..2000 {
        let o = rng.point(30.0);
        let target = rng.point(10.0);
        let d = [target[0] - o[0], target[1] - o[1], target[2] - o[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let d = [d[0] / l, d[1] / l, d[2] / l];
        let got = bvh.intersect(&tris, o, d, 100.0).map(|h| (h.triangle, h.t));
        let want = brute(&tris, o, d, 100.0);
        assert_eq!(got.map(|g| g.0), want.map(|w| w.0), "ray {o:?} {d:?}");
        assert_eq!(bvh.occluded(&tris, o, d, 100.0), want.is_some());
        hits += want.is_some() as u32;
    }
    assert!(hits > 200, "{hits}");
}

#[test]
fn closest_point_matches_brute_force() {
    let mut rng = Rng(0xfeed_beef);
    let tris = soup(800, &mut rng);
    let bvh = Bvh::build(&tris);
    for _ in 0..500 {
        let p = rng.point(30.0);
        let (_, _, d) = bvh.closest(&tris, p, 1.0e9).unwrap();
        let want = tris
            .iter()
            .map(|t| {
                let q = t.closest_point(p);
                ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt()
            })
            .fold(f32::INFINITY, f32::min);
        assert!((d - want).abs() < 1.0e-4, "{d} vs {want}");
    }
}

#[test]
fn one_sided_triangles_are_missed_from_behind() {
    // Counter-clockwise seen from +z: the front faces +z.
    let t = MeshTriangle::flat(
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        0,
        false,
    );
    assert_eq!(t.face_normal(), [0.0, 0.0, 1.0]);
    let bvh = Bvh::build(&[t]);
    assert!(bvh
        .intersect(&[t], [0.2, 0.2, 1.0], [0.0, 0.0, -1.0], 10.0)
        .is_some());
    assert!(bvh
        .intersect(&[t], [0.2, 0.2, -1.0], [0.0, 0.0, 1.0], 10.0)
        .is_none());
    let two = MeshTriangle {
        two_sided: true,
        ..t
    };
    let hit = bvh
        .intersect(&[two], [0.2, 0.2, -1.0], [0.0, 0.0, 1.0], 10.0)
        .unwrap();
    assert!(hit.back && (hit.t - 1.0).abs() < 1e-6);
    assert!(Bvh::build(&[])
        .intersect(&[], [0.0; 3], [1.0, 0.0, 0.0], 1.0)
        .is_none());
}
