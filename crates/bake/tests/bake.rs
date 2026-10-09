//! SDF, cards and the cache on small meshes with known answers.

use std::path::Path;

use genos_bake::{bake, load_or_bake, read, BakeError, BakeMesh, BakeMode, BakeSettings};
use genos_load::{load_gltf_file, Image, PbrMaterial, TextureSlot};
use genos_mesh::MeshTriangle;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures")
        .join(name)
}

fn settings(dir: &str) -> BakeSettings {
    BakeSettings {
        mode: BakeMode::OnLoad,
        cache_dir: std::env::temp_dir()
            .join(format!("genos-bake-test-{dir}-{}", std::process::id())),
        card_texel: 0.1,
        ..BakeSettings::default()
    }
}

/// An axis-aligned box from -h to +h, outward faces.
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

#[test]
fn closed_mesh_sdf_is_signed_and_accurate_near_the_surface() {
    let scene = load_gltf_file(fixture("cube.gltf")).unwrap();
    let mesh = BakeMesh::from_gltf(&scene, 0);
    let b = bake(&mesh, &settings("sdf"));
    let sdf = &b.sdf;
    assert!(!sdf.thin);
    assert!((sdf.voxel - 2.0 / 64.0).abs() < 1e-6);
    assert!(sdf.distance([0.0; 3]) < -0.9 * sdf.band, "centre is inside");
    assert!(sdf.distance([3.0, 0.0, 0.0]) > 0.0, "far outside");
    // Near the surface the nearest-voxel value is within a voxel of the truth.
    for (p, want) in [
        ([1.05, 0.1, -0.2], 0.05),
        ([0.95, 0.3, 0.4], -0.05),
        ([0.2, -1.08, 0.1], 0.08),
        ([-0.3, 0.2, 0.93], -0.07),
    ] {
        let got = sdf.distance(p);
        assert!((got - want).abs() < sdf.voxel, "{p:?}: {got} vs {want}");
    }
    // Only the shell is stored: far fewer bricks than the grid.
    let total = sdf.bricks_dims.iter().product::<u32>() as usize;
    assert!(sdf.bricks.len() < total, "{} of {total}", sdf.bricks.len());
}

#[test]
fn open_mesh_is_thin_and_unsigned() {
    let quad = vec![
        MeshTriangle::flat(
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            0,
            false,
        ),
        MeshTriangle::flat(
            [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            0,
            false,
        ),
    ];
    let mesh = BakeMesh::plain("quad", quad, &[[0.8; 3]]);
    let b = bake(&mesh, &settings("thin"));
    assert!(b.sdf.thin);
    for z in [-0.02f32, 0.02] {
        let d = b.sdf.distance([0.5, 0.5, z]);
        assert!(d >= 0.0 && d < 0.04, "{z}: {d}");
    }
}

#[test]
fn cards_capture_albedo_normal_depth_and_coverage() {
    let mesh = BakeMesh::plain("box", cube(0.5), &[[0.5, 0.25, 1.0]]);
    let b = bake(&mesh, &settings("cards"));
    assert_eq!(b.cards.cards.len(), 6);
    for c in &b.cards.cards {
        assert_eq!((c.width, c.height), (10, 10), "1 m at 0.1 m texels");
        for t in &c.texels {
            assert_eq!(t.albedo, [128, 64, 255, 255]);
            assert!(t.depth < 100, "the face is on the card plane");
        }
    }
    // +x card sees normals along +x.
    let n = b.cards.cards[0].texels[0].normal;
    assert!(
        n[0] > 250 && (n[1] as i32 - 128).abs() < 2 && (n[2] as i32 - 128).abs() < 2,
        "{n:?}"
    );
}

#[test]
fn cards_keep_the_power_of_texture_driven_emission() {
    // A 1 x 1 m quad facing +z; the left half of its emissive texture is white.
    let quad = vec![
        MeshTriangle {
            uvs: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            ..MeshTriangle::flat(
                [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                0,
                false,
            )
        },
        MeshTriangle {
            uvs: [[0.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            ..MeshTriangle::flat(
                [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                0,
                false,
            )
        },
    ];
    let mut pixels = Vec::new();
    for _y in 0..8 {
        for x in 0..8 {
            pixels.extend(if x < 4 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            });
        }
    }
    let images = [Image {
        width: 8,
        height: 8,
        pixels,
    }];
    let slot = TextureSlot {
        image: 0,
        texcoord: 0,
        sampler: Default::default(),
        scale: 1.0,
    };
    let mut mesh = BakeMesh::plain("panel", quad, &[[0.8; 3]]);
    mesh.images = &images;
    mesh.materials[0].pbr = PbrMaterial {
        emissive: [2.0, 1.0, 0.5],
        emissive_texture: Some(slot),
        emissive_strength: 3.0,
        metallic: 0.0,
        ..PbrMaterial::default()
    };
    let b = bake(&mesh, &settings("emissive"));
    // +z card (face 4): half the area at 3 x factor.
    let p = b.cards.emitted(4);
    for (got, want) in p.iter().zip([3.0, 1.5, 0.75]) {
        assert!((got - want).abs() < want * 0.02, "{p:?}");
    }
    // Lit texels only on the left half.
    let c = b.cards.cards.iter().find(|c| c.face == 4).unwrap();
    let left = c.texels[(c.height / 2 * c.width + 1) as usize].emissive[0];
    let right = c.texels[(c.height / 2 * c.width + c.width - 2) as usize].emissive[0];
    assert!(left > 5.9 && right == 0.0, "{left} {right}");
}

#[test]
fn cache_modes_and_keys() {
    let mesh = BakeMesh::plain("box", cube(0.5), &[[0.8; 3]]);
    let mut s = settings("cache");
    let _ = std::fs::remove_dir_all(&s.cache_dir);
    s.mode = BakeMode::Prebuilt;
    assert!(matches!(
        load_or_bake(&mesh, &s),
        Err(BakeError::Missing { .. })
    ));
    s.mode = BakeMode::OnLoad;
    let first = load_or_bake(&mesh, &s).unwrap();
    let path = genos_bake::cache_path(&mesh, &s);
    assert_eq!(read(&path).as_ref(), Some(&first), "the file round-trips");
    s.mode = BakeMode::Prebuilt;
    assert_eq!(load_or_bake(&mesh, &s).unwrap(), first);
    let mut other = s.clone();
    other.card_texel = 0.05;
    assert_ne!(mesh.content_key(&s), mesh.content_key(&other));
    let tinted = BakeMesh::plain("box", cube(0.5), &[[0.7; 3]]);
    assert_ne!(mesh.content_key(&s), tinted.content_key(&s));
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 3);
    std::fs::write(&path, bytes).unwrap();
    assert!(read(&path).is_none(), "a cut file is ignored");
    let _ = std::fs::remove_dir_all(&s.cache_dir);
}
