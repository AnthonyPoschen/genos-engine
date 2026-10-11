//! Print what `load_model_file` reads from a glTF, GLB, FBX or OBJ file.
//! `cargo run -p genos-load --example model_info -- FILE...`

fn main() {
    for path in std::env::args().skip(1) {
        match genos_load::load_model_file(&path) {
            Err(e) => println!("{path}: {e}"),
            Ok(scene) => {
                let mut lo = [f32::MAX; 3];
                let mut hi = [f32::MIN; 3];
                let mut tris = 0;
                for inst in &scene.instances {
                    for p in &scene.meshes[inst.mesh].primitives {
                        tris += p.indices.len() / 3;
                        for v in &p.positions {
                            let w = genos_load::transform_point(&inst.transform, *v);
                            for k in 0..3 {
                                lo[k] = lo[k].min(w[k]);
                                hi[k] = hi[k].max(w[k]);
                            }
                        }
                    }
                }
                println!(
                    "{path}: {} meshes, {} instances, {} triangles, {} materials, {} images, bounds {:?} .. {:?}",
                    scene.meshes.len(),
                    scene.instances.len(),
                    tris,
                    scene.materials.len(),
                    scene.images.len(),
                    lo,
                    hi
                );
            }
        }
    }
}
