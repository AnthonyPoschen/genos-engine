#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! Stage 1 discrete LOD chain (geometry-lod design): cook determinism, monotone
//! triangle counts, the two-sided object-space error bound, locked borders,
//! selector hysteresis, and a planted selector that drops the depth term.

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

/// Vertices, edge midpoints, and face interiors of `from`, measured onto `onto`.
fn max_surface_deviation(from: &Primitive, onto: &Primitive) -> f32 {
    let mut worst = 0.0_f32;
    for point in surface_points(from) {
        worst = worst.max(min_distance(point, onto));
    }
    worst
}

fn two_sided(source: &Primitive, level: &Primitive) -> f32 {
    max_surface_deviation(source, level).max(max_surface_deviation(level, source))
}

fn surface_points(prim: &Primitive) -> Vec<[f32; 3]> {
    let interiors = [
        [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
        [0.5, 0.25, 0.25],
        [0.25, 0.5, 0.25],
        [0.25, 0.25, 0.5],
    ];
    let mut out = Vec::new();
    for tri in prim.indices.as_chunks::<3>().0 {
        let Some(a) = vertex(prim, tri[0]) else {
            continue;
        };
        let Some(b) = vertex(prim, tri[1]) else {
            continue;
        };
        let Some(c) = vertex(prim, tri[2]) else {
            continue;
        };
        out.extend([a, b, c, mid(a, b), mid(b, c), mid(c, a)]);
        for weights in interiors {
            out.push(bary(a, b, c, weights));
        }
    }
    out
}

fn min_distance(point: [f32; 3], prim: &Primitive) -> f32 {
    let mut best = f32::MAX;
    for tri in prim.indices.as_chunks::<3>().0 {
        let Some(a) = vertex(prim, tri[0]) else {
            continue;
        };
        let Some(b) = vertex(prim, tri[1]) else {
            continue;
        };
        let Some(c) = vertex(prim, tri[2]) else {
            continue;
        };
        best = best.min(dist_point_triangle(point, a, b, c));
    }
    best
}

fn mid(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        f32::midpoint(a[0], b[0]),
        f32::midpoint(a[1], b[1]),
        f32::midpoint(a[2], b[2]),
    ]
}

fn bary(a: [f32; 3], b: [f32; 3], c: [f32; 3], w: [f32; 3]) -> [f32; 3] {
    [
        a[0] * w[0] + b[0] * w[1] + c[0] * w[2],
        a[1] * w[0] + b[1] * w[1] + c[1] * w[2],
        a[2] * w[0] + b[2] * w[1] + c[2] * w[2],
    ]
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

fn view_at(depth_m: f32, error_px: f32, bias: i32) -> LodView {
    LodView {
        depth_m,
        scale: 1.0,
        height_px: 1000.0,
        fov_y_rad: std::f32::consts::FRAC_PI_2,
        error_px,
        bias,
    }
}

/// `e * H / (2 d tan(fov/2))`, written out so it does not call the selector.
fn projected(error_m: f32, depth_m: f32, fov_y_rad: f32) -> f32 {
    error_m * 1000.0 / (2.0 * depth_m * (fov_y_rad * 0.5).tan())
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
        let measured = two_sided(source, &level.primitives[0]);
        if i == 0 {
            // The imported surface against itself. Closest-point noise only.
            assert!(measured <= 1.0e-5, "level 0 self deviation {measured}");
        } else {
            assert!(
                measured <= level.error_m,
                "level {i}: surface deviation {measured} ({}) exceeds stored {} ({})",
                measured.to_bits(),
                level.error_m,
                level.error_m.to_bits()
            );
        }
        previous = level.error_m;
    }
    let coarsest = chain.last().unwrap();
    let measured = two_sided(source, &coarsest.primitives[0]);
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
    let fov = std::f32::consts::FRAC_PI_2;
    let at_boundary = view_at(1000.0, tau, 0);
    assert!(projected(2.0, 1000.0, fov) - tau < 1.0e-4);
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
    assert!(projected(0.5, 400.0, fov) < tau);
    assert!(projected(2.0, 400.0, fov) > tau);
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
    let mutant = select_lod_without_distance(&errors, far);
    let chosen_tris = counts.get(chosen).copied().unwrap();
    let mutant_tris = counts.get(mutant).copied().unwrap();
    assert!(
        chosen_tris < mutant_tris,
        "far level {chosen} ({chosen_tris} tris) must be coarser than the depth-dropped mutant level {mutant} ({mutant_tris} tris)"
    );
}

/// The screen-error formula with the depth term removed. A positive error no
/// longer shrinks as the instance moves away, so the far pick stays finer.
fn select_lod_without_distance(errors_m: &[f32], view: LodView) -> usize {
    let mut best = 0;
    for (index, error) in errors_m.iter().copied().enumerate() {
        let screen = if error <= 0.0 || view.scale <= 0.0 {
            0.0
        } else {
            let denom = 2.0 * (view.fov_y_rad * 0.5).tan();
            if !error.is_finite() || !denom.is_finite() || denom <= 0.0 {
                f32::INFINITY
            } else {
                error * view.scale * view.height_px / denom
            }
        };
        if screen < view.error_px {
            best = index;
        }
    }
    best
}

/// Two grids of one continuous sheet, split at x = 1. The shared edge is a
/// ridge of the sine, not a crease, so an unlocked border can move it.
fn shared_edge(cells: usize) -> GltfMesh {
    GltfMesh {
        name: Some(String::from("pair")),
        primitives: vec![sheet(cells, 0.0, 1.0), sheet(cells, 1.0, 2.0)],
    }
}

fn sheet(cells: usize, x0: f32, x1: f32) -> Primitive {
    let n = cells + 1;
    let mut positions = Vec::with_capacity(n * n);
    let span = unit(cells);
    for y in 0..n {
        for x in 0..n {
            let fx = unit(x) / span;
            let fy = unit(y) / span;
            let world_x = x0 + (x1 - x0) * fx;
            let z = (world_x * std::f32::consts::FRAC_PI_2).sin()
                * (fy * std::f32::consts::PI).sin()
                * 0.25;
            positions.push([world_x, fy, z]);
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
    Primitive {
        positions,
        indices,
        ..Primitive::default()
    }
}

fn seam_vertices(prim: &Primitive) -> Vec<[f32; 3]> {
    let max_x = prim
        .positions
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max);
    prim.positions
        .iter()
        .copied()
        .filter(|point| point[0].to_bits() == max_x.to_bits())
        .collect()
}

#[test]
fn locked_borders_leave_no_gap_along_a_shared_edge() {
    let mesh = shared_edge(16);
    let chain = lod_chain(&mesh);
    let seam = seam_vertices(&mesh.primitives[0]);
    assert!(seam.len() > 2, "shared edge has {} vertices", seam.len());
    assert_eq!(chain[0].primitives.len(), 2);
    for (level_index, level) in chain.iter().enumerate() {
        let mut gap = 0.0_f32;
        for point in &seam {
            gap = gap
                .max(min_distance(*point, &level.primitives[0]))
                .max(min_distance(*point, &level.primitives[1]));
        }
        assert_eq!(
            gap.to_bits(),
            0.0_f32.to_bits(),
            "level {level_index} seam gap {gap} m"
        );
    }
}

/// Body is `GCK1` plus a little-endian version word of 1. The trailer is the
/// SHA-256 of that body, so the reader gets past the hash and rejects the version.
fn version_one_record() -> Vec<u8> {
    let mut body = b"GCK1".to_vec();
    body.extend_from_slice(&1u32.to_le_bytes());
    let trailer = genos_load::sha256(&body);
    body.extend_from_slice(&trailer);
    body
}

#[test]
fn a_version_1_record_is_rejected() {
    let bytes = version_one_record();
    assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    let load_err = genos_load::decode_geometry(&bytes).unwrap_err();
    assert!(
        load_err.to_string().contains("version"),
        "loader: {load_err}"
    );
    let cook_err = genos_cook::decode(&bytes).unwrap_err();
    assert!(cook_err.to_string().contains("version"), "cook: {cook_err}");
}
