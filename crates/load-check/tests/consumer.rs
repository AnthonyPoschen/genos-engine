//! Calls the public `genos-load` API from a second package.
//!
//! This package does not open a window or a Vulkan device.

use std::path::{Path, PathBuf};

use genos_load::{
    load_animation, load_font, load_image, load_material, load_mesh, load_pcm, load_texture,
    load_texture_map, FileSource, MemorySource,
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

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures")
        .join(name)
}

#[test]
fn loads_png_texture_map_mesh_material_animation_wav_and_font() {
    let png = std::fs::read(fixture("quad.png")).expect("png fixture");
    let from_memory = load_image(&MemorySource::new(&png)).expect("memory png");
    let from_path = load_image(&FileSource::new(fixture("quad.png"))).expect("path png");
    assert_eq!(from_memory, from_path);
    assert_eq!(from_memory.width, 2);
    assert_eq!(from_memory.height, 2);
    assert_eq!(from_memory.pixels, QUAD);
    assert_eq!(from_memory.pixel(0, 0), Some([255, 0, 0, 255]));

    let texture = load_texture(&MemorySource::new(&png), "hero").expect("texture");
    assert_eq!(texture.name, "hero");
    assert_eq!(texture.image.pixels, QUAD);
    assert_eq!(texture.pixel(1, 0), Some([0, 255, 0, 255]));

    let map = load_texture_map(&MemorySource::new(&png), &MemorySource::new(MAP.as_bytes()))
        .expect("map");
    assert!(map.clips.len() >= 2);
    let idle_start = map.sample("idle", 0.0).expect("idle start");
    let idle_next = map.sample("idle", 1.0).expect("idle next");
    let walk_start = map.sample("walk", 0.0).expect("walk");
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
    assert_eq!((walk_start.x, walk_start.y), (0, 1));
    assert!(idle_start.x + idle_start.width <= map.image.width);
    assert!(idle_next.y + idle_next.height <= map.image.height);
    assert!(walk_start.x + walk_start.width <= map.image.width);
    assert!(walk_start.y + walk_start.height <= map.image.height);

    let mesh = load_mesh(&MemorySource::new(OBJ.as_bytes())).expect("mesh");
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

    let material = load_material(&MemorySource::new(MATERIAL.as_bytes())).expect("material");
    assert_eq!(material.color, [0.25, 0.5, 0.75]);
    assert_eq!(material.texture.as_deref(), Some("stone.png"));

    let animation = load_animation(&MemorySource::new(ANIMATION.as_bytes())).expect("animation");
    let pose = animation.sample(1.0).expect("halfway");
    assert_eq!(pose.translation, [2.0, 3.0, 4.0]);
    assert_eq!(pose.scale, [2.0, 3.0, 4.0]);
    assert_ne!(pose.rotation, animation.keys[0].pose.rotation);

    let wav_bytes = wav(22050, 2, &[0, 1000, -1000, 32767]);
    let pcm = load_pcm(&MemorySource::new(&wav_bytes)).expect("wav");
    assert_eq!(pcm.sample_rate, 22050);
    assert_eq!(pcm.channels, 2);
    assert_eq!(pcm.samples, [0, 1000, -1000, 32767]);

    let font_bytes = std::fs::read(fixture("glyph.ttf")).expect("font fixture");
    let font = load_font(&MemorySource::new(&font_bytes)).expect("font");
    let glyph = font.glyph('A').expect("glyph");
    assert_eq!(font.units_per_em(), 1024);
    assert_eq!(glyph.advance, 800);
    assert!(!glyph.contours.is_empty());
    let em = i32::from(font.units_per_em());
    for contour in &glyph.contours {
        assert!(!contour.is_empty());
        for point in contour {
            assert!(point.x >= 0 && point.x <= em);
            assert!(point.y >= 0 && point.y <= em);
        }
    }
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
