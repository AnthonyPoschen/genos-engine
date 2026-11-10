//! `genos-animation`: the animation debug binary. Headless import dump.
//! See `debug/animation/README.md`.

use std::process::ExitCode;

use genos_animation_debug::dump;
use genos_load::import_file;

const USAGE: &str = "genos-animation: headless animation debug binary (import dump, no draw)

  <file.gltf|file.glb>   print one JSON object and exit
  -h, --help             this text

The object lists skins (joint name, parent, node, notes), clips (name, duration,
channels), the influence count, the node skin index, and the import notes.
A missing inverse bind, a reduced fifth weight, and an omitted cubic-spline
sampler show up in skin_notes. There is no window and no Vulkan device.

Later commands (not in this binary yet): --anim-play, MCP anim.state, and
--measure-skin on the owner's PC.";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        println!("{USAGE}");
        return Ok(());
    };
    if path == "-h" || path == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    if args.next().is_some() {
        return Err(format!("one file only\n\n{USAGE}"));
    }
    let doc = import_file(&path).map_err(|err| format!("{path}: {err}"))?;
    println!("{}", genos_json::encode(&dump(&doc)));
    Ok(())
}
