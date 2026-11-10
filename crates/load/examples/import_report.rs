//! Print the import notes of glTF files: `cargo run -p genos-load --example import_report -- scene.glb`.

fn main() {
    for path in std::env::args().skip(1) {
        match genos_load::import_file(&path) {
            Ok(doc) => {
                println!("{path}: asset {}", genos_load::hex(&doc.asset_id));
                for note in &doc.notes {
                    println!("  {note}");
                }
            }
            Err(err) => println!("{path}: error {err}"),
        }
    }
}
