//! Public load entry points. Each test starts at `load_*` with bytes or a path.

use std::path::{Path, PathBuf};

use genos_load::{
    load_animation, load_font, load_image, load_material, load_mesh, load_pcm, load_texture,
    load_texture_map, FileSource, LoadError, MemorySource, OutlinePoint,
};

const QUAD: [u8; 16] = [
    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
];

const MAP: &str = "\
frame idle_0 0 0 1 1
frame idle_1 1 0 1 1
frame walk_0 0 1 1 1
frame walk_1 1 1 1 1
clip idle 1 idle_0 idle_1
clip walk 1 walk_0 walk_1
";

const OBJ: &str = "\
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
vn 0 0 1
vn 0 1 0
vn 1 0 0
usemtl stone
f 1/1/1 2/2/2 3/3/3
";

const MATERIAL: &str = "\
color 0.25 0.5 0.75
texture stone.png
";

const ANIMATION: &str = "\
key 0 0 0 0 0 0 0 1 1 1 1
key 2 4 6 8 0 0.70710677 0 0.70710677 3 5 7
";

const SHORT_ARC: &str = "\
key 0 0 0 0 0 0 0 1 1 1 1
key 1 0 0 0 0 -0.17364818 0 -0.98480775 1 1 1
";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn quad_png() -> Vec<u8> {
    std::fs::read(fixture("quad.png")).expect("quad fixture")
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "genos-load-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, bytes).expect("temp file");
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn path_and_memory_decode_the_same_png() {
    let bytes = quad_png();
    let from_memory = load_image(&MemorySource::new(&bytes)).expect("memory png");
    let file = TempFile::new(&bytes);
    let from_path = load_image(&FileSource::new(&file.0)).expect("path png");
    assert_eq!(from_memory, from_path);
    assert_eq!(from_memory.width, 2);
    assert_eq!(from_memory.height, 2);
    assert_eq!(from_memory.pixels, QUAD);
}

#[test]
fn png_filters_reconstruct_the_original_pixels() {
    let sub = png(2, 1, 6, &[1, 10, 20, 30, 40, 2, 5, 3, 10], &[], &[]);
    let image = load_image(&MemorySource::new(&sub)).expect("sub");
    assert_eq!(image.pixels, [10, 20, 30, 40, 12, 25, 33, 50]);

    let average = png(1, 2, 6, &[0, 10, 20, 30, 40, 3, 11, 20, 25, 30], &[], &[]);
    let image = load_image(&MemorySource::new(&average)).expect("average");
    assert_eq!(image.pixels, [10, 20, 30, 40, 16, 30, 40, 50]);

    let paeth = png(
        2,
        2,
        6,
        &[0, 10, 0, 0, 0, 10, 0, 0, 0, 4, 70, 0, 0, 0, 10, 0, 0, 0],
        &[],
        &[],
    );
    let image = load_image(&MemorySource::new(&paeth)).expect("paeth");
    assert_eq!(
        image.pixels,
        [10, 0, 0, 0, 10, 0, 0, 0, 80, 0, 0, 0, 90, 0, 0, 0]
    );
}

#[test]
fn indexed_png_uses_the_palette_and_transparency() {
    let bytes = png(2, 1, 3, &[0, 0, 1], &[255, 0, 0, 0, 0, 255], &[128, 255]);
    let image = load_image(&MemorySource::new(&bytes)).expect("indexed");
    assert_eq!(image.pixel(0, 0), Some([255, 0, 0, 128]));
    assert_eq!(image.pixel(1, 0), Some([0, 0, 255, 255]));
}

#[test]
fn texture_keeps_the_name_and_pixels() {
    let bytes = quad_png();
    let texture = load_texture(&MemorySource::new(&bytes), "hero").expect("texture");
    assert_eq!(texture.name, "hero");
    assert_eq!(texture.image.width, 2);
    assert_eq!(texture.image.height, 2);
    assert_eq!(texture.pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(texture.pixel(1, 1), Some([255, 255, 255, 255]));
    assert_eq!(texture.image.pixels, QUAD);
}

#[test]
fn texture_map_samples_two_clips_across_a_frame_boundary() {
    let map = load_texture_map(
        &MemorySource::new(&quad_png()),
        &MemorySource::new(MAP.as_bytes()),
    )
    .expect("map");
    assert!(map.clips.len() >= 2);
    assert_eq!(map.clips[0].name, "idle");
    assert_eq!(map.clips[1].name, "walk");
    let idle_start = map.sample("idle", 0.0).expect("idle start");
    let idle_before = map.sample("idle", 0.999).expect("idle before");
    let idle_next = map.sample("idle", 1.0).expect("idle next");
    let walk_start = map.sample("walk", 0.0).expect("walk start");
    assert_eq!(idle_start, idle_before);
    assert_ne!(idle_start, idle_next);
    assert_eq!(
        (
            idle_start.x,
            idle_start.y,
            idle_start.width,
            idle_start.height
        ),
        (0, 0, 1, 1)
    );
    assert_eq!(
        (idle_next.x, idle_next.y, idle_next.width, idle_next.height),
        (1, 0, 1, 1)
    );
    assert_eq!(
        (
            walk_start.x,
            walk_start.y,
            walk_start.width,
            walk_start.height
        ),
        (0, 1, 1, 1)
    );
    for rect in [idle_start, idle_next, walk_start] {
        assert!(rect.x + rect.width <= map.image.width);
        assert!(rect.y + rect.height <= map.image.height);
    }
}

#[test]
fn obj_triangle_keeps_positions_uvs_normals_and_material() {
    let mesh = load_mesh(&MemorySource::new(OBJ.as_bytes())).expect("obj");
    assert_eq!(mesh.triangles.len(), 1);
    let triangle = &mesh.triangles[0];
    assert_eq!(
        triangle.positions,
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    );
    assert_eq!(
        triangle.texcoords,
        Some([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]])
    );
    assert_eq!(
        triangle.normals,
        Some([[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]])
    );
    assert_eq!(triangle.material.as_deref(), Some("stone"));
}

#[test]
fn material_keeps_color_and_texture_file() {
    let material = load_material(&MemorySource::new(MATERIAL.as_bytes())).expect("material");
    assert_eq!(material.color, [0.25, 0.5, 0.75]);
    assert_eq!(material.texture.as_deref(), Some("stone.png"));

    let commented = load_material(&MemorySource::new(b"# note\ncolor 1 0 0\n")).expect("comment");
    assert_eq!(commented.color, [1.0, 0.0, 0.0]);
    assert_eq!(commented.texture, None);
}

#[test]
fn animation_halfway_translation_and_scale_are_midpoints() {
    let animation = load_animation(&MemorySource::new(ANIMATION.as_bytes())).expect("animation");
    assert_eq!(animation.keys.len(), 2);
    let pose = animation.sample(1.0).expect("sample");
    assert_eq!(pose.translation, [2.0, 3.0, 4.0]);
    assert_eq!(pose.scale, [2.0, 3.0, 4.0]);
    let start = animation.keys[0].pose.rotation;
    let end = animation.keys[1].pose.rotation;
    assert!(pose.rotation != start);
    assert!(pose.rotation != end);
    assert!(pose.rotation[1] > 0.2 && pose.rotation[1] < 0.55);
    assert!(pose.rotation[3] > 0.85 && pose.rotation[3] < 0.98);
    let length = pose
        .rotation
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt();
    assert!((length - 1.0).abs() < 1.0e-4);
}

#[test]
fn animation_rotation_takes_the_short_arc() {
    let animation = load_animation(&MemorySource::new(SHORT_ARC.as_bytes())).expect("short arc");
    let pose = animation.sample(0.5).expect("sample");
    let start = animation.keys[0].pose.rotation;
    let dot = pose.rotation[0] * start[0]
        + pose.rotation[1] * start[1]
        + pose.rotation[2] * start[2]
        + pose.rotation[3] * start[3];
    assert!(dot > 0.9);
    assert!(pose.rotation != start);
}

#[test]
fn wav_mono_and_stereo_samples_match() {
    let mono = wav(44100, 1, &[1, -2, 3]);
    let decoded = load_pcm(&MemorySource::new(&mono)).expect("mono");
    assert_eq!(decoded.sample_rate, 44100);
    assert_eq!(decoded.channels, 1);
    assert_eq!(decoded.samples, [1, -2, 3]);

    let stereo = wav(22050, 2, &[0, 1000, -1000, 32767]);
    let file = TempFile::new(&stereo);
    let from_path = load_pcm(&FileSource::new(&file.0)).expect("stereo path");
    let from_memory = load_pcm(&MemorySource::new(&stereo)).expect("stereo memory");
    assert_eq!(from_path, from_memory);
    assert_eq!(from_memory.sample_rate, 22050);
    assert_eq!(from_memory.channels, 2);
    assert_eq!(from_memory.samples, [0, 1000, -1000, 32767]);
}

#[test]
fn font_glyph_advance_and_outline_lie_in_the_em_square() {
    let bytes = std::fs::read(fixture("glyph.ttf")).expect("font fixture");
    let file = TempFile::new(&bytes);
    let from_path = load_font(&FileSource::new(&file.0)).expect("font path");
    let from_memory = load_font(&MemorySource::new(&bytes)).expect("font memory");
    assert_eq!(from_path.units_per_em(), from_memory.units_per_em());
    let glyph = from_memory.glyph('A').expect("glyph");
    let path_glyph = from_path.glyph('A').expect("path glyph");
    assert_eq!(glyph, path_glyph);
    assert_eq!(glyph.advance, 800);
    assert_eq!(from_memory.units_per_em(), 1024);
    assert!(!glyph.contours.is_empty());
    let points: Vec<OutlinePoint> = glyph.contours.into_iter().flatten().collect();
    assert!(!points.is_empty());
    let em = i32::from(from_memory.units_per_em());
    for point in &points {
        assert!(point.x >= 0 && point.x <= em);
        assert!(point.y >= 0 && point.y <= em);
    }
    assert_eq!(
        points,
        vec![
            OutlinePoint {
                x: 100,
                y: 100,
                on_curve: true
            },
            OutlinePoint {
                x: 700,
                y: 100,
                on_curve: true
            },
            OutlinePoint {
                x: 700,
                y: 800,
                on_curve: true
            },
            OutlinePoint {
                x: 100,
                y: 800,
                on_curve: true
            },
        ]
    );
}

#[test]
fn missing_truncated_and_unknown_bytes_are_errors() {
    let missing = std::env::temp_dir().join(format!("genos-load-missing-{}", std::process::id()));
    let err = load_image(&FileSource::new(&missing)).expect_err("missing");
    assert!(matches!(err, LoadError::NotFound { .. }));

    let png = quad_png();
    let err = load_image(&MemorySource::new(&png[..12])).expect_err("truncated png");
    assert_eq!(err, LoadError::Truncated);
    let err = load_image(&MemorySource::new(b"not a png")).expect_err("unknown png");
    assert_eq!(err, LoadError::Unrecognized);

    let stereo = wav(22050, 2, &[0, 1000, -1000, 32767]);
    let err = load_pcm(&MemorySource::new(&stereo[..20])).expect_err("truncated wav");
    assert_eq!(err, LoadError::Truncated);
    let err = load_pcm(&MemorySource::new(b"hello")).expect_err("unknown wav");
    assert_eq!(err, LoadError::Unrecognized);

    let err = load_mesh(&MemorySource::new(b"this is not a mesh")).expect_err("unknown obj");
    assert_eq!(err, LoadError::Unrecognized);
    let err = load_font(&MemorySource::new(b"OTTO not truetype")).expect_err("unknown font");
    assert_eq!(err, LoadError::Unrecognized);
    let font = std::fs::read(fixture("glyph.ttf")).expect("font");
    let err = load_font(&MemorySource::new(&font[..20])).expect_err("truncated font");
    assert_eq!(err, LoadError::Truncated);
}

fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let align = channels * 2;
    let mut data = Vec::new();
    for sample in samples {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    fmt.extend_from_slice(&align.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let riff_size = 4 + 8 + fmt.len() + 8 + data.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(riff_size as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    out.extend_from_slice(&fmt);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn png(
    width: u32,
    height: u32,
    color: u8,
    raw: &[u8],
    palette: &[u8],
    transparency: &[u8],
) -> Vec<u8> {
    let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &ihdr));
    if !palette.is_empty() {
        out.extend(png_chunk(b"PLTE", palette));
    }
    if !transparency.is_empty() {
        out.extend(png_chunk(b"tRNS", transparency));
    }
    out.extend(png_chunk(b"IDAT", &stored_zlib(raw)));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn png_chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let mut crc_bytes = tag.to_vec();
    crc_bytes.extend_from_slice(body);
    out.extend_from_slice(&(!crc32(&crc_bytes)).to_be_bytes());
    out
}

fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut offset = 0usize;
    if data.is_empty() {
        out.push(0x01);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0xffffu16.to_le_bytes());
    }
    while offset < data.len() {
        let end = (offset + 65535).min(data.len());
        let last = end == data.len();
        let chunk = &data[offset..end];
        out.push(if last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
        offset = end;
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}
