//! Load images, textures, meshes, animations, audio samples, and fonts.
//!
//! Decoders read bytes. They do not open a window, an audio device, or a socket.
//! [`FileSource`] and [`MemorySource`] share one decode path.

mod animation;
mod bin;
mod error;
mod gltf;
mod inflate;
mod jpeg;
mod material;
mod mesh;
mod png;
mod source;
mod texture;
mod ttf;
mod wav;

pub use animation::{Animation, Transform, TransformKey};
pub use error::LoadError;
pub use gltf::{
    transform_point, AlphaMode, GltfMesh, GltfScene, MeshInstance, PbrMaterial, Primitive, Sampler,
    TextureSlot,
};
pub use material::Material;
pub use mesh::{Mesh, Triangle};
pub use png::Image;
pub use source::{ByteSource, FileSource, MemorySource};
pub use texture::{MapClip, MapFrame, Rect, Texture, TextureMap};
pub use ttf::{Font, Glyph, OutlinePoint};
pub use wav::Pcm;

fn read(source: &dyn ByteSource) -> Result<Vec<u8>, LoadError> {
    source.read_bytes()
}

/// Decode a glTF 2.0 scene (`.gltf` or `.glb`). `resolve` reads a URI relative
/// to the file (external buffers and images); `data:` URIs never reach it.
pub fn load_gltf(
    source: &dyn ByteSource,
    resolve: &dyn Fn(&str) -> Result<Vec<u8>, LoadError>,
) -> Result<GltfScene, LoadError> {
    gltf::decode_gltf(&read(source)?, resolve)
}

/// Decode a glTF 2.0 file; relative URIs are read from the file's directory.
pub fn load_gltf_file(path: impl AsRef<std::path::Path>) -> Result<GltfScene, LoadError> {
    let path = path.as_ref();
    let dir = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    load_gltf(&FileSource::new(path), &|uri| {
        FileSource::new(dir.join(uri)).read_bytes()
    })
}

/// Decode a PNG or a JPEG into an RGBA8 image. The format comes from the bytes.
pub fn load_image(source: &dyn ByteSource) -> Result<Image, LoadError> {
    decode_image(&read(source)?)
}

/// Decode PNG or JPEG bytes into an RGBA8 image.
pub fn decode_image(bytes: &[u8]) -> Result<Image, LoadError> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg::decode_jpeg(bytes)
    } else {
        png::decode_png(bytes)
    }
}

/// Decode a PNG or a JPEG and keep `name` as the texture's reference.
pub fn load_texture(source: &dyn ByteSource, name: &str) -> Result<Texture, LoadError> {
    if name.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    Ok(Texture {
        name: name.to_string(),
        image: load_image(source)?,
    })
}

/// Decode one image and a texture-map description.
pub fn load_texture_map(
    image: &dyn ByteSource,
    description: &dyn ByteSource,
) -> Result<TextureMap, LoadError> {
    let image = load_image(image)?;
    texture::decode_map(image, &read(description)?)
}

/// Decode a Wavefront OBJ into triangles.
pub fn load_mesh(source: &dyn ByteSource) -> Result<Mesh, LoadError> {
    mesh::decode_obj(&read(source)?)
}

/// Decode a text material description.
pub fn load_material(source: &dyn ByteSource) -> Result<Material, LoadError> {
    material::decode_material(&read(source)?)
}

/// Decode a text animation clip.
pub fn load_animation(source: &dyn ByteSource) -> Result<Animation, LoadError> {
    animation::decode_animation(&read(source)?)
}

/// Decode a 16-bit PCM WAV.
pub fn load_pcm(source: &dyn ByteSource) -> Result<Pcm, LoadError> {
    wav::decode_wav(&read(source)?)
}

/// Decode a TrueType font.
pub fn load_font(source: &dyn ByteSource) -> Result<Font, LoadError> {
    ttf::decode_font(&read(source)?)
}
