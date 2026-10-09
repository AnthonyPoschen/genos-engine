//! Print what a glTF file contains: `cargo run -p genos-load --example gltf_info -- scene.glb`.

fn main() {
    for path in std::env::args().skip(1) {
        let start = std::time::Instant::now();
        match genos_load::load_gltf_file(&path) {
            Ok(scene) => {
                let prims: usize = scene.meshes.iter().map(|m| m.primitives.len()).sum();
                let texels: u64 = scene
                    .images
                    .iter()
                    .map(|i| i.width as u64 * i.height as u64)
                    .sum();
                println!(
                    "{path}: {} meshes, {prims} primitives, {} instances, {} world triangles, {} materials, {} images ({:.1} Mtexels), {:.0} ms",
                    scene.meshes.len(),
                    scene.instances.len(),
                    scene.world_triangles().len(),
                    scene.materials.len(),
                    scene.images.len(),
                    texels as f64 / 1e6,
                    start.elapsed().as_secs_f64() * 1e3
                );
            }
            Err(err) => println!("{path}: {err}"),
        }
    }
}
