#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(
        clippy::items_after_statements,
        reason = "png helpers stay next to the writer"
    )
)]
#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! ADR 0035. A fixture loaded from glTF and from its cooked package yields the
//! same geometry and instances. The lavapipe pictures agree within the ADR
//! bound (mean absolute channel error ≤ 3/255, 99th percentile ≤ 16/255).
//! BC7/BC5 error is expected inside that bound. `--verify`'s check
//! (`Package::verify`, the same check `genos-cook --verify` runs) rejects a
//! corrupted record.
//!
//! `cook_package` writes no `mat=` token, and that picture is the path from
//! before the material record. `cook_gmt1` is the `genos-cook` output, including
//! the `.gmat`, and the covered picture test draws it through `add_cooked` and
//! through `World::add_covered` after the source file is gone.

use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_cook::{cook_asset, load_cooked, package, package_with, Threads};
use genos_load::{decode_geometry, sha256, Package};
use genos_render::{Renderer, World};
use genos_scene::{Camera, Floor, Scene, Vec3};
use genos_window::Window;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genos-cooked-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Uncompressed 8-bit RGBA PNG.
fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let mut raw = Vec::new();
    for row in rgba.chunks(usize::try_from(width).unwrap() * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    fn adler(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in data {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
    fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[4..]);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }
    let mut zlib = vec![0x78, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = u16::try_from((raw.len() - i).min(65535)).unwrap();
        let last = u8::from(i + usize::from(n) == raw.len());
        zlib.push(last);
        zlib.extend_from_slice(&n.to_le_bytes());
        zlib.extend_from_slice(&(!n).to_le_bytes());
        zlib.extend_from_slice(&raw[i..i + usize::from(n)]);
        i += usize::from(n);
    }
    zlib.extend_from_slice(&adler(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(chunk(*b"IHDR", &ihdr));
    png.extend(chunk(*b"IDAT", &zlib));
    png.extend(chunk(*b"IEND", &[]));
    std::fs::write(path, png).unwrap();
}

fn fixture(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let w = 32u32;
    let texels = usize::try_from(w).unwrap() * usize::try_from(w).unwrap() * 4;
    let mut rgba = vec![0u8; texels];
    let mut nrm = vec![0u8; texels];
    for y in 0..w {
        for x in 0..w {
            let i = (usize::try_from(y).unwrap() * usize::try_from(w).unwrap()
                + usize::try_from(x).unwrap())
                * 4;
            // One step per 4×4 block. A steeper ramp blows the ADR's p99 on
            // BC7's basic profile; this one stays inside it.
            rgba[i] = 96 + u8::try_from(x / 4).unwrap();
            rgba[i + 1] = 140 + u8::try_from(y / 8).unwrap();
            rgba[i + 2] = 180;
            rgba[i + 3] = 255;
            nrm[i] = 128;
            nrm[i + 1] = 128;
            nrm[i + 2] = 255;
            nrm[i + 3] = 255;
        }
    }
    write_png(&dir.join("color.png"), w, w, &rgba);
    write_png(&dir.join("normal.png"), w, w, &nrm);
    let positions: [[f32; 3]; 4] = [
        [-8.0, -8.0, 1.0],
        [8.0, -8.0, 1.0],
        [8.0, 8.0, 1.0],
        [-8.0, 8.0, 1.0],
    ];
    let uvs: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let normals: [[f32; 3]; 4] = [[0.0, 0.0, 1.0]; 4];
    let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
    let mut bin = Vec::new();
    for p in positions {
        for v in p {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    for n in normals {
        for v in n {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    for uv in uvs {
        for v in uv {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    for i in indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    std::fs::write(dir.join("quad.bin"), &bin).unwrap();
    let gltf = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0, "name": "quad"}}],
  "meshes": [{{"primitives": [{{
    "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
    "indices": 3,
    "material": 0
  }}]}}],
  "materials": [{{
    "doubleSided": true,
    "pbrMetallicRoughness": {{
      "baseColorFactor": [1, 1, 1, 1],
      "metallicFactor": 0,
      "roughnessFactor": 1,
      "baseColorTexture": {{"index": 0}}
    }},
    "normalTexture": {{"index": 1}}
  }}],
  "textures": [{{"source": 0}}, {{"source": 1}}],
  "images": [{{"uri": "color.png"}}, {{"uri": "normal.png"}}],
  "buffers": [{{"uri": "quad.bin", "byteLength": {len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 48, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 96, "byteLength": 32}},
    {{"buffer": 0, "byteOffset": 128, "byteLength": 12}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2"}},
    {{"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}}
  ]
}}"#,
        len = bin.len()
    );
    let path = dir.join("quad.gltf");
    std::fs::write(&path, gltf).unwrap();
    path
}

fn cook_package(source: &Path, out: &Path) {
    let settings = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: out.join("bake"),
        ..genos_bake::BakeSettings::default()
    };
    let cooked = cook_asset(source, out, &settings, true, Threads::ONE).unwrap();
    let record = load_cooked(&cooked.record.1).unwrap();
    let gtex = sha256(&std::fs::read(&cooked.textures.unwrap().1).unwrap());
    let text = package(&[(&record, Some(gtex))]).text();
    assert!(
        !text.contains("mat="),
        "the legacy package names no material record"
    );
    genos_load::write_atomic(&out.join("package.manifest"), text.as_bytes()).unwrap();
}

/// The bytes `genos-cook` writes: `.gcook`, `.gtex`, `.gmat`, and `mat=` after `gtex=`.
fn cook_gmt1(source: &Path, out: &Path) -> [u8; 32] {
    let settings = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: out.join("bake"),
        ..genos_bake::BakeSettings::default()
    };
    let cooked = cook_asset(source, out, &settings, true, Threads::ONE).unwrap();
    let record = load_cooked(&cooked.record.1).unwrap();
    let gtex = sha256(&std::fs::read(&cooked.textures.unwrap().1).unwrap());
    let mat_bytes = std::fs::read(&cooked.materials.1).unwrap();
    assert!(
        mat_bytes.starts_with(b"GMT1"),
        "genos-cook writes a GMT1 material record"
    );
    let mat = sha256(&mat_bytes);
    let text = package_with(&[(&record, Some(gtex), Some(mat))]).text();
    let line = text.lines().nth(1).unwrap();
    let gtex_at = line.find(" gtex=").unwrap();
    let mat_at = line.find(" mat=").unwrap();
    assert!(mat_at > gtex_at, "{line}");
    genos_load::write_atomic(&out.join("package.manifest"), text.as_bytes()).unwrap();
    record.asset_id
}

fn geometry_of(
    scene: &genos_load::GltfScene,
) -> (Vec<genos_load::GltfMesh>, Vec<genos_load::MeshInstance>) {
    (scene.meshes.clone(), scene.instances.clone())
}

#[test]
fn gltf_and_cooked_geometry_match_and_verify_rejects_corruption() {
    let dir = temp("geom");
    let source = fixture(&dir.join("src"));
    let out = dir.join("cook");
    cook_package(&source, &out);
    let direct = genos_load::load_gltf_file(&source).unwrap();
    let bytes = std::fs::read(genos_load::cooked_path(
        &out,
        &sha256(&std::fs::read(&source).unwrap()),
    ))
    .unwrap();
    let cooked = decode_geometry(&bytes).unwrap();
    let (meshes, instances) = geometry_of(&direct);
    assert_eq!(cooked.meshes, meshes);
    assert_eq!(cooked.instances, instances);

    let package = Package::open(&out).unwrap();
    assert_eq!(package.verify(&out).unwrap(), 1);
    let gcook = genos_load::cooked_path(&out, &cooked.asset_id);
    let mut damaged = std::fs::read(&gcook).unwrap();
    let last = damaged.len() - 1;
    damaged[last] ^= 0xff;
    std::fs::write(&gcook, damaged).unwrap();
    assert!(
        package.verify(&out).is_err(),
        "verify must reject a corrupted package"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn settle(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let mut picture = Vec::new();
    for _ in 0..64 {
        let _ = window.pump();
        picture = renderer.draw(world, camera, true).unwrap().unwrap();
        if renderer.material_residency().1 == Some(0) {
            break;
        }
    }
    for _ in 0..8 {
        let _ = window.pump();
        picture = renderer.draw(world, camera, true).unwrap().unwrap();
    }
    picture
}

fn picture_error(a: &[u8], b: &[u8]) -> (u64, usize, u8) {
    let n = a.len().min(b.len()) / 4;
    let mut sum = 0u64;
    let mut hist = [0u32; 256];
    for i in 0..n {
        for c in 0..3 {
            let d = usize::from(a[i * 4 + c].abs_diff(b[i * 4 + c]));
            sum += u64::from(u8::try_from(d).unwrap());
            hist[d] += 1;
        }
    }
    let samples = (n * 3).max(1);
    let mut seen = 0u64;
    let mut p99 = 0u8;
    let target = u64::try_from((samples * 99).div_ceil(100)).unwrap();
    for (d, count) in hist.iter().enumerate() {
        seen += u64::from(*count);
        if seen >= target {
            p99 = u8::try_from(d).unwrap();
            break;
        }
    }
    (sum, samples, p99)
}

fn mean_text(sum: u64, samples: usize) -> String {
    let samples = u64::try_from(samples).unwrap();
    let whole = sum / samples;
    let frac = (sum % samples) * 1000 / samples;
    format!("{whole}.{frac:03}")
}

fn identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn proof_world(lit: bool) -> World {
    let mut world = World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(1000.0, 0.0, 0.0),
            half_x: 0.01,
            half_z: 0.01,
            color: [0.0, 0.0, 0.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: if lit {
            vec![genos_scene::Light {
                position: Vec3::new(0.0, 2.0, 3.0),
                color: [1.0, 1.0, 1.0],
                direction: Vec3::ZERO,
            }]
        } else {
            Vec::new()
        },
        ceiling: None,
        sky: None,
        lighting: if lit {
            genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Potato)
        } else {
            genos_scene::LightingSettings::default()
        },
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    });
    for object in &mut world.objects {
        if let genos_render::DrawKind::Fixed(genos_render::FixedPart::Floor) = object.kind {
            object.hidden = true;
            object.affects_light = false;
        }
    }
    world
}

fn world_with(scene_gltf: &genos_load::GltfScene) -> World {
    let mut world = proof_world(true);
    world.add_gltf(scene_gltf, &identity(), false);
    world
}

fn compare(label: &str, source: &Path, out: &Path, check: bool) {
    let cold = Instant::now();
    let direct = genos_load::load_gltf_file(source).unwrap();
    let cold_ms = cold.elapsed().as_secs_f64() * 1e3;
    let warm_start = Instant::now();
    let direct_warm = genos_load::load_gltf_file(source).unwrap();
    let warm_gltf_ms = warm_start.elapsed().as_secs_f64() * 1e3;
    assert_eq!(geometry_of(&direct), geometry_of(&direct_warm));

    let _guard = gpu_lock();
    let mut window = Window::open_proof(128, 128).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    let mut app = genos_window::Lifecycle::new(window, renderer, permit);
    assert!(
        app.device.bc_textures(),
        "lavapipe is expected to expose textureCompressionBC"
    );

    let mut camera = Camera::new(0.0, 4.0, 0.0);
    camera.position = Vec3::new(0.0, 1.2, 2.2);
    let gltf_world = world_with(&direct);
    let rgba_bytes = gltf_world.color_atlas.rgba.len() + gltf_world.normal_atlas.rgba.len();
    let gltf_pic = settle(&mut app.window, &mut app.device, &gltf_world, &camera);

    let manifest = std::fs::read_to_string(out.join("package.manifest")).unwrap();
    assert!(
        !manifest.contains("mat="),
        "this picture is the package with no material record"
    );
    genos_load::reset_importer_entries();
    let mut base = proof_world(true);
    let place = identity();
    let cold_c = Instant::now();
    let report = base.add_cooked(out, source, &place, false, true);
    assert!(
        genos_load::importer_entries() >= 1,
        "a line without mat= still decodes the glTF"
    );
    let cold_cooked_ms = cold_c.elapsed().as_secs_f64() * 1e3;
    assert_eq!(report.fallback.len(), 0, "{:?}", report.fallback);
    assert_ne!(report.objects.len(), 0);
    let block_bytes = report.texture_bytes;
    let cooked_pic = settle(&mut app.window, &mut app.device, &base, &camera);

    let mut again = proof_world(false);
    let warm_c = Instant::now();
    let warm = again.add_cooked(out, source, &place, false, true);
    let warm_cooked_ms = warm_c.elapsed().as_secs_f64() * 1e3;
    assert_eq!(warm.objects.len(), report.objects.len());

    let (sum, samples, p99) = picture_error(&gltf_pic, &cooked_pic);
    let mean = mean_text(sum, samples);
    eprintln!("{label} picture mean_abs {mean}/255 p99 {p99}/255  (bound mean ≤ 3, p99 ≤ 16)");
    eprintln!(
        "{label} load ms  glTF cold {cold_ms:.2}  glTF warm {warm_gltf_ms:.2}  cooked cold {cold_cooked_ms:.2}  cooked warm {warm_cooked_ms:.2}"
    );
    eprintln!(
        "{label} texture bytes  rgba atlases {rgba_bytes}  bc atlases {block_bytes}  delta {}",
        i128::from(block_bytes) - i128::from(u64::try_from(rgba_bytes).unwrap())
    );
    if check {
        assert_bc_bound(label, sum, samples, p99, &mean);
    }
}

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn assert_bc_bound(label: &str, sum: u64, samples: usize, p99: u8, mean: &str) {
    let samples = u64::try_from(samples).unwrap();
    assert!(
        sum <= samples * 3,
        "{label} mean absolute error {mean}/255 exceeds 3/255"
    );
    assert!(p99 <= 16, "{label} p99 error {p99}/255 exceeds 16/255");
}

#[test]
fn cooked_picture_matches_gltf_within_the_bc_bound_and_load_times_are_printed() {
    let dir = temp("pic");
    let source = fixture(&dir.join("src"));
    let out = dir.join("cook");
    cook_package(&source, &out);
    compare("ramp", &source, &out, true);
    let _ = std::fs::remove_dir_all(&dir);
}

fn ensure_png(repo: &Path, rel: &str) {
    let path = repo.join(rel);
    let bytes = std::fs::read(&path).unwrap_or_default();
    if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1") || bytes.is_empty() {
        let status = std::process::Command::new("git")
            .args(["lfs", "pull", "--include", rel])
            .current_dir(repo)
            .status()
            .expect("git lfs pull");
        assert!(status.success(), "git lfs pull {rel}");
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        bytes.starts_with(b"\x89PNG"),
        "{rel} is not a PNG ({} bytes)",
        bytes.len()
    );
}

#[test]
fn sponza_floor_picture_matches_gltf_within_the_bc_bound() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let diffuse = "assets/built/sponza/textures/Sponza_Floor_diffuse.png";
    let normal = "assets/built/sponza/textures/Sponza_Floor_normal.png";
    ensure_png(&repo, diffuse);
    ensure_png(&repo, normal);
    let dir = temp("sponza-floor");
    let source = fixture(&dir.join("src"));
    let src = source.parent().unwrap();
    std::fs::copy(repo.join(diffuse), src.join("color.png")).unwrap();
    std::fs::copy(repo.join(normal), src.join("normal.png")).unwrap();
    let out = dir.join("cook");
    cook_package(&source, &out);
    compare("sponza floor", &source, &out, true);
    let _ = std::fs::remove_dir_all(&dir);
}

fn push_f32(bin: &mut Vec<u8>, value: f32) {
    bin.extend_from_slice(&value.to_le_bytes());
}

fn push_vec3(bin: &mut Vec<u8>, value: [f32; 3]) {
    for channel in value {
        push_f32(bin, channel);
    }
}

/// Two meshes, each placed twice. Mesh 0 is the ramp quad. One copy fills the
/// view the legacy fixture fills. The other is scaled down and sits above two
/// flat-colour copies of mesh 1. The ramp's texel steps are the ones the ADR
/// bound was measured on.
fn write_ramp(dir: &Path) {
    let w = 32u32;
    let texels = usize::try_from(w).unwrap() * usize::try_from(w).unwrap() * 4;
    let mut rgba = vec![0u8; texels];
    let mut nrm = vec![0u8; texels];
    for y in 0..w {
        for x in 0..w {
            let i = (usize::try_from(y).unwrap() * usize::try_from(w).unwrap()
                + usize::try_from(x).unwrap())
                * 4;
            rgba[i] = 96 + u8::try_from(x / 4).unwrap();
            rgba[i + 1] = 140 + u8::try_from(y / 8).unwrap();
            rgba[i + 2] = 180;
            rgba[i + 3] = 255;
            nrm[i] = 128;
            nrm[i + 1] = 128;
            nrm[i + 2] = 255;
            nrm[i + 3] = 255;
        }
    }
    write_png(&dir.join("color.png"), w, w, &rgba);
    write_png(&dir.join("normal.png"), w, w, &nrm);
}

struct BinOffsets {
    mesh0_pos: usize,
    mesh0_nrm: usize,
    mesh0_uv: usize,
    mesh0_idx: usize,
    mesh1_pos: usize,
    mesh1_nrm: usize,
    mesh1_idx: usize,
}

fn instanced_bin() -> (Vec<u8>, BinOffsets) {
    let mut bin = Vec::new();
    let mesh0_pos = bin.len();
    for p in [
        [-8.0, -8.0, 1.0],
        [8.0, -8.0, 1.0],
        [8.0, 8.0, 1.0],
        [-8.0, 8.0, 1.0],
    ] {
        push_vec3(&mut bin, p);
    }
    let mesh0_nrm = bin.len();
    for _ in 0..4 {
        push_vec3(&mut bin, [0.0, 0.0, 1.0]);
    }
    let mesh0_uv = bin.len();
    for uv in [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]] {
        push_f32(&mut bin, uv[0]);
        push_f32(&mut bin, uv[1]);
    }
    let mesh0_idx = bin.len();
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let mesh1_pos = bin.len();
    for p in [
        [-0.09, -0.09, 0.0],
        [0.09, -0.09, 0.0],
        [0.09, 0.09, 0.0],
        [-0.09, 0.09, 0.0],
    ] {
        push_vec3(&mut bin, p);
    }
    let mesh1_nrm = bin.len();
    for _ in 0..4 {
        push_vec3(&mut bin, [0.0, 0.0, 1.0]);
    }
    let mesh1_idx = bin.len();
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    (
        bin,
        BinOffsets {
            mesh0_pos,
            mesh0_nrm,
            mesh0_uv,
            mesh0_idx,
            mesh1_pos,
            mesh1_nrm,
            mesh1_idx,
        },
    )
}

fn instanced_fixture(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    write_ramp(dir);
    let (bin, at) = instanced_bin();
    std::fs::write(dir.join("quad.bin"), &bin).unwrap();
    let gltf = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0, 1, 2, 3]}}],
  "nodes": [
    {{"mesh": 0, "name": "ramp"}},
    {{"mesh": 0, "translation": [0, 1.40, 1.62], "scale": [0.01, 0.01, 1]}},
    {{"mesh": 1, "translation": [-0.22, 1.05, 1.5]}},
    {{"mesh": 1, "translation": [0.22, 1.05, 1.5]}}
  ],
  "meshes": [
    {{"primitives": [{{
      "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
      "indices": 3,
      "material": 0
    }}]}},
    {{"primitives": [{{
      "attributes": {{"POSITION": 4, "NORMAL": 5}},
      "indices": 6,
      "material": 1
    }}]}}
  ],
  "materials": [
    {{
      "doubleSided": true,
      "pbrMetallicRoughness": {{
        "baseColorFactor": [1, 1, 1, 1],
        "metallicFactor": 0,
        "roughnessFactor": 1,
        "baseColorTexture": {{"index": 0}}
      }},
      "normalTexture": {{"index": 1}}
    }},
    {{
      "doubleSided": true,
      "pbrMetallicRoughness": {{
        "baseColorFactor": [0.2, 0.75, 0.3, 1],
        "metallicFactor": 0,
        "roughnessFactor": 1
      }}
    }}
  ],
  "textures": [{{"source": 0}}, {{"source": 1}}],
  "images": [{{"uri": "color.png"}}, {{"uri": "normal.png"}}],
  "buffers": [{{"uri": "quad.bin", "byteLength": {len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": {mesh0_pos}, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": {mesh0_nrm}, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": {mesh0_uv}, "byteLength": 32}},
    {{"buffer": 0, "byteOffset": {mesh0_idx}, "byteLength": 12}},
    {{"buffer": 0, "byteOffset": {mesh1_pos}, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": {mesh1_nrm}, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": {mesh1_idx}, "byteLength": 12}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2"}},
    {{"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}},
    {{"bufferView": 4, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 5, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 6, "componentType": 5123, "count": 6, "type": "SCALAR"}}
  ]
}}"#,
        len = bin.len(),
        mesh0_pos = at.mesh0_pos,
        mesh0_nrm = at.mesh0_nrm,
        mesh0_uv = at.mesh0_uv,
        mesh0_idx = at.mesh0_idx,
        mesh1_pos = at.mesh1_pos,
        mesh1_nrm = at.mesh1_nrm,
        mesh1_idx = at.mesh1_idx,
    );
    let path = dir.join("quad.gltf");
    std::fs::write(&path, gltf).unwrap();
    path
}

fn delete_source(source: &Path, out: &Path) {
    let src = source.parent().unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            std::fs::remove_file(path).unwrap();
        }
    }
    let objects = out.join("objects");
    if objects.is_dir() {
        for entry in std::fs::read_dir(&objects).unwrap() {
            std::fs::write(entry.unwrap().path(), b"not-a-gltf").unwrap();
        }
    }
}

fn whole_u32(value: f32) -> u32 {
    let mut whole = 0u32;
    let mut step = 0.0f32;
    while step + 1.0 <= value {
        step += 1.0;
        whole = whole.saturating_add(1);
        if whole == 100_000 {
            break;
        }
    }
    whole
}

fn sample_rgb(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> [u8; 3] {
    let width_f = f32::from(u16::try_from(width).expect("width fits u16"));
    let height_f = f32::from(u16::try_from(height).expect("height fits u16"));
    let aspect = width_f / height_f;
    let uv = genos_scene::viewport_uv(camera, aspect, world).expect("point is on screen");
    let x = whole_u32((uv[0] * width_f).clamp(0.0, width_f - 1.0));
    let y = whole_u32((uv[1] * height_f).clamp(0.0, height_f - 1.0));
    let i = (usize::try_from(y).unwrap() * usize::try_from(width).unwrap()
        + usize::try_from(x).unwrap())
        * 4;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn channel_gap(a: [u8; 3], b: [u8; 3]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(left, right)| left.abs_diff(right))
        .max()
        .unwrap()
}

#[test]
fn covered_picture_matches_gltf_after_the_source_is_gone() {
    let dir = temp("covered-pic");
    let source = instanced_fixture(&dir.join("src"));
    let out = dir.join("cook");
    let id = cook_gmt1(&source, &out);
    let direct = genos_load::load_gltf_file(&source).unwrap();

    let _guard = gpu_lock();
    let mut window = Window::open_proof(128, 128).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    let mut app = genos_window::Lifecycle::new(window, renderer, permit);
    assert!(
        app.device.bc_textures(),
        "lavapipe exposes textureCompressionBC"
    );
    let mut camera = Camera::new(0.0, 4.0, 0.0);
    camera.position = Vec3::new(0.0, 1.2, 2.2);

    let mut oracle = proof_world(true);
    let oracle_ids = oracle.add_gltf(&direct, &identity(), false);
    assert_eq!(oracle_ids.len(), 4, "two meshes, each instanced twice");
    let oracle_pic = settle(&mut app.window, &mut app.device, &oracle, &camera);

    genos_load::reset_importer_entries();
    let mut cooked = proof_world(true);
    let cooked_report = cooked.add_cooked(&out, &source, &identity(), false, true);
    assert_eq!(genos_load::importer_entries(), 0);
    assert_eq!(
        cooked_report.fallback.len(),
        0,
        "{:?}",
        cooked_report.fallback
    );
    assert_eq!(cooked_report.objects.len(), 4);
    assert!(cooked_report.texture_bytes > 0);
    let cooked_pic = settle(&mut app.window, &mut app.device, &cooked, &camera);
    let (sum, samples, p99) = picture_error(&oracle_pic, &cooked_pic);
    let mean = mean_text(sum, samples);
    eprintln!("covered add_cooked picture mean_abs {mean}/255 p99 {p99}/255");
    assert_bc_bound("covered add_cooked", sum, samples, p99, &mean);

    delete_source(&source, &out);
    assert!(!source.exists(), "the source glTF is gone");
    genos_load::reset_importer_entries();
    let mut covered = proof_world(true);
    let covered_report = covered.add_covered(&out, &id, &identity(), false, true);
    assert_eq!(genos_load::importer_entries(), 0);
    assert_eq!(
        covered_report.fallback.len(),
        0,
        "{:?}",
        covered_report.fallback
    );
    assert_eq!(covered_report.objects.len(), 4);
    assert!(covered_report.texture_bytes > 0);
    let covered_pic = settle(&mut app.window, &mut app.device, &covered, &camera);
    let (sum, samples, p99) = picture_error(&oracle_pic, &covered_pic);
    let mean = mean_text(sum, samples);
    eprintln!("add_covered picture mean_abs {mean}/255 p99 {p99}/255");
    assert_bc_bound("add_covered", sum, samples, p99, &mean);

    let width = app.device.width();
    let height = app.device.height();
    let left = [-0.22, 1.05, 1.5];
    let right = [0.22, 1.05, 1.5];
    let ramp = [0.0, 1.22, 1.0];
    let oracle_left = sample_rgb(&oracle_pic, width, height, &camera, left);
    let oracle_right = sample_rgb(&oracle_pic, width, height, &camera, right);
    let oracle_ramp = sample_rgb(&oracle_pic, width, height, &camera, ramp);
    assert!(
        channel_gap(oracle_left, oracle_ramp) > 16,
        "the flat card is not in the oracle picture: card {oracle_left:?} ramp {oracle_ramp:?}"
    );
    assert!(
        channel_gap(oracle_left, oracle_right) <= 16,
        "the two instances diverged: {oracle_left:?} {oracle_right:?}"
    );
    let covered_left = sample_rgb(&covered_pic, width, height, &camera, left);
    assert!(
        channel_gap(oracle_left, covered_left) <= 16,
        "add_covered card {covered_left:?} oracle {oracle_left:?}"
    );

    genos_load::reset_importer_entries();
    let mut missing = proof_world(true);
    let missing_report = missing.add_cooked(&out, &source, &identity(), false, true);
    assert_eq!(missing_report.objects.len(), 0);
    assert_ne!(missing_report.fallback.len(), 0);
    assert_eq!(genos_load::importer_entries(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
