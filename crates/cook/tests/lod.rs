#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! Stage 1 discrete LOD chain (geometry-lod design): cook determinism, monotone
//! triangle counts, the object-space error bound, selector hysteresis, and a
//! negative control that drops the distance term.

use genos_bake::{BakeMode, BakeSettings};
use genos_cook::{cook, encode, lod_chain};
use genos_load::{select_lod, GltfMesh, GltfScene, ImportDoc, LodView, Primitive, LOD_HYSTERESIS};

const RATIOS: [f64; 3] = [0.5, 0.25, 0.125];

fn unit(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

fn wavy(cells: usize) -> GltfMesh {
    let n = cells + 1;
    let mut positions = Vec::with_capacity(n * n);
    let span = unit(cells);
    for y in 0..n {
        for x in 0..n {
            let fx = unit(x) / span;
            let fy = unit(y) / span;
            let z = (fx * std::f32::consts::PI).sin() * (fy * std::f32::consts::PI).sin() * 0.25;
            positions.push([fx, fy, z]);
        }
    }
    let mut indices = Vec::with_capacity(cells * cells * 6);
    for y in 0..cells {
        for x in 0..cells {
            let i = u32::try_from(y * n + x).unwrap();
            let row = u32::try_from(n).unwrap();
            indices.extend([i, i + 1, i + row, i + 1, i + row + 1, i + row]);
        }
    }
    GltfMesh {
        name: Some(String::from("wavy")),
        primitives: vec![Primitive {
            positions,
            indices,
            ..Primitive::default()
        }],
    }
}

fn triangles(mesh: &GltfMesh) -> usize {
    mesh.primitives.iter().map(|p| p.indices.len() / 3).sum()
}

fn level_tris(level: &genos_load::MeshLod) -> usize {
    level.primitives.iter().map(|p| p.indices.len() / 3).sum()
}

fn grid_doc(mesh: GltfMesh) -> ImportDoc {
    let source = b"wavy-grid-v1".to_vec();
    ImportDoc {
        asset_id: genos_load::sha256(&source),
        source,
        externals: Vec::new(),
        scene: Some(GltfScene {
            meshes: vec![mesh],
            ..GltfScene::default()
        }),
        notes: Vec::new(),
        kept: Vec::new(),
    }
}

fn settings(dir: &std::path::Path) -> BakeSettings {
    BakeSettings {
        mode: BakeMode::OnLoad,
        cache_dir: dir.join("bake"),
        voxels_per_side: 8.0,
        ..BakeSettings::default()
    }
}

/// Object-space distance from every level-0 vertex to the level's triangles.
fn max_vertex_deviation(source: &Primitive, level: &Primitive) -> f32 {
    let mut worst = 0.0_f32;
    for p in &source.positions {
        let mut best = f32::MAX;
        for tri in level.indices.as_chunks::<3>().0 {
            let Some(a) = vertex(level, tri[0]) else {
                continue;
            };
            let Some(b) = vertex(level, tri[1]) else {
                continue;
            };
            let Some(c) = vertex(level, tri[2]) else {
                continue;
            };
            best = best.min(dist_point_triangle(*p, a, b, c));
        }
        worst = worst.max(best);
    }
    worst
}

fn vertex(prim: &Primitive, index: u32) -> Option<[f32; 3]> {
    prim.positions.get(usize::try_from(index).ok()?).copied()
}

#[allow(
    clippy::many_single_char_names,
    reason = "Ericson, Real-Time Collision Detection 5.1.5 uses these names"
)]
fn dist_point_triangle(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return len(ap);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return len(bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return len(sub(ap, scale(ab, v)));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return len(cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return len(sub(ap, scale(ac, w)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return len(sub(bp, scale(sub(c, b), w)));
    }
    let denom = va + vb + vc;
    if denom.abs() <= 1.0e-20 {
        return len(ap).min(len(bp)).min(len(cp));
    }
    let v = vb / denom;
    let w = vc / denom;
    len(sub(ap, add(scale(ab, v), scale(ac, w))))
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn view_at(distance_m: f32, error_px: f32, bias: i32) -> LodView {
    LodView {
        distance_m,
        height_px: 1000.0,
        fov_y_rad: std::f32::consts::FRAC_PI_2,
        error_px,
        bias,
    }
}

/// `e * H / (2 d tan(fov/2))`, written out so it does not call the selector.
fn projected(error_m: f32, distance_m: f32) -> f32 {
    error_m * 1000.0 / (2.0 * distance_m)
}

#[test]
fn two_cooks_of_the_same_mesh_are_the_same_bytes() {
    let dir = std::env::temp_dir().join(format!("genos-lod-det-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = grid_doc(wavy(16));
    let s = settings(&dir);
    let first = cook(&doc, &s).unwrap();
    let second = cook(&doc, &s).unwrap();
    let bytes = encode(&first);
    assert_eq!(bytes, encode(&second));
    let loaded = genos_cook::decode(&bytes).unwrap();
    assert_eq!(loaded, first);
    let geom = genos_load::decode_geometry(&bytes).unwrap();
    assert_eq!(geom.meshes[0], first.meshes[0].mesh);
    assert_eq!(geom.lods[0].len(), first.meshes[0].lods.len());
    for (a, b) in geom.lods[0].iter().zip(&first.meshes[0].lods) {
        assert_eq!(a.error_m.to_bits(), b.error_m.to_bits());
        assert_eq!(level_tris(a), level_tris(b));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn triangle_counts_are_monotone_and_near_the_targets() {
    let mesh = wavy(32);
    let chain = lod_chain(&mesh);
    assert_eq!(chain.len(), 4);
    assert_eq!(chain[0].error_m.to_bits(), 0.0_f32.to_bits());
    assert_eq!(chain[0].primitives, mesh.primitives);
    let lod0 = triangles(&mesh);
    let mut previous = lod0;
    for (level, ratio) in chain.iter().skip(1).zip(RATIOS) {
        let count = level_tris(level);
        assert!(count <= previous, "{count} tris after {previous}");
        let got = f64::from(u32::try_from(count).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(lod0).unwrap_or(1));
        let tol = 0.10 * ratio;
        assert!(
            (got - ratio).abs() <= tol,
            "ratio {got} is outside {ratio} ± {tol}"
        );
        previous = count;
    }
}

#[test]
fn stored_error_bounds_the_deviation_and_is_monotone() {
    let mesh = wavy(32);
    let chain = lod_chain(&mesh);
    let source = &mesh.primitives[0];
    let mut previous = 0.0_f32;
    for (i, level) in chain.iter().enumerate() {
        assert!(level.error_m.is_finite() && level.error_m >= previous);
        let measured = max_vertex_deviation(source, &level.primitives[0]);
        let slack = 1.0e-4_f32.max(level.error_m * 1.0e-3);
        assert!(
            measured <= level.error_m + slack,
            "level {i}: deviation {measured} exceeds stored {} + {slack}",
            level.error_m
        );
        previous = level.error_m;
    }
    let coarsest = chain.last().unwrap();
    let measured = max_vertex_deviation(source, &coarsest.primitives[0]);
    assert!(
        measured > 1.0e-4 && coarsest.error_m > 0.0,
        "coarsest error {} deviation {measured} should be a real bound",
        coarsest.error_m
    );
}

#[test]
fn selection_holds_across_the_boundary_and_switches_past_the_band() {
    // screen = e * 1000 / (2 d) with fov 90°. Level 2 (error 2) crosses τ = 1 at d = 1000.
    let errors = [0.0, 0.5, 2.0];
    let tau = 1.0;
    let at_boundary = view_at(1000.0, tau, 0);
    assert!(projected(2.0, 1000.0) - tau < 1.0e-4);
    assert_eq!(select_lod(&errors, at_boundary, None), 1);
    assert_eq!(select_lod(&errors, at_boundary, Some(1)), 1);
    assert_eq!(select_lod(&errors, at_boundary, Some(2)), 2);

    let mut held = select_lod(&errors, at_boundary, None);
    for distance in [1001.0, 999.0, 1000.5, 999.5, 1010.0] {
        let next = select_lod(&errors, view_at(distance, tau, 0), Some(held));
        assert_eq!(next, held, "pop at distance {distance}");
    }

    let clear = 1000.0 / (1.0 - LOD_HYSTERESIS);
    let back = 1000.0 / (1.0 + LOD_HYSTERESIS);
    assert_eq!(
        select_lod(&errors, view_at(clear + 1.0, tau, 0), Some(1)),
        2
    );
    assert_eq!(select_lod(&errors, view_at(back - 1.0, tau, 0), Some(2)), 1);
    held = select_lod(&errors, view_at(clear + 1.0, tau, 0), None);
    assert_eq!(held, 2);
    for distance in [1000.0, 999.0, back + 1.0] {
        assert_eq!(
            select_lod(&errors, view_at(distance, tau, 0), Some(2)),
            2,
            "coarse side popped at {distance}"
        );
    }

    // Low preset bias is one level coarser than the error pick.
    // At d = 400, level 1 is under τ and level 2 is not.
    assert!(projected(0.5, 400.0) < tau);
    assert!(projected(2.0, 400.0) > tau);
    assert_eq!(select_lod(&errors, view_at(400.0, tau, 0), None), 1);
    assert_eq!(select_lod(&errors, view_at(400.0, tau, 1), None), 2);
}

#[test]
fn negative_control_dropping_the_distance_term_fails_the_far_triangle_oracle() {
    let mesh = wavy(32);
    let chain = lod_chain(&mesh);
    let errors: Vec<f32> = chain.iter().map(|level| level.error_m).collect();
    let counts: Vec<usize> = chain.iter().map(level_tris).collect();
    let lod0 = counts[0];
    let far = view_at(1.0e5, 1.0, 0);
    let chosen = select_lod(&errors, far, None);
    let fewer = |level: usize| counts.get(level).is_some_and(|n| *n < lod0);
    assert!(
        fewer(chosen),
        "far level {chosen} tris {:?} should be under {lod0}",
        counts.get(chosen)
    );
    let mutant = 0;
    assert!(
        !fewer(mutant),
        "dropping the distance term stays on LOD0 and must fail the far-triangle oracle"
    );
}
