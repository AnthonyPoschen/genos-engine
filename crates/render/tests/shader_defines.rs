#![cfg_attr(
    test,
    expect(
        clippy::panic,
        reason = "a test aborts when a production shader define fails to compile"
    )
)]

//! Every define combination `crates/render/build.rs` ships, plus
//! `GENOS_COMPOSE_NO_WALK` on `texel_detail_prod.comp`.
//!
//! Picture compose always defines `GENOS_COMPOSE_NO_WALK`. The lavapipe fixture
//! does not, and it calls `lighting_sh_read` and `lighting_sh_detail_store`.
//! Those helpers live in the shared cache include. A compile that omits the
//! define stays green while the define fails. This test runs the compilers
//! the build uses, with the same flags.

use std::path::PathBuf;
use std::process::Command;

struct ShaderCompile {
    label: &'static str,
    compiler: &'static str,
    src: &'static str,
    flags: &'static [&'static str],
}

/// `glslc` flags match `compile_args`. `glslangValidator` flags match
/// `compile_defines` and `compile_restir_variant`.
const PRODUCTION: &[ShaderCompile] = &[
    ShaderCompile {
        label: "compose",
        compiler: "glslc",
        src: "shaders/lighting_compose.comp",
        flags: &["-DGENOS_COMPOSE_NO_WALK=1"],
    },
    ShaderCompile {
        label: "compose hdr",
        compiler: "glslangValidator",
        src: "shaders/lighting_compose.comp",
        flags: &[
            "-V",
            "--target-env",
            "vulkan1.0",
            "-DGENOS_HDR_COMPOSE",
            "-DGENOS_COMPOSE_NO_WALK",
        ],
    },
    ShaderCompile {
        label: "compose maint",
        compiler: "glslc",
        src: "shaders/lighting_compose.comp",
        flags: &["-DGENOS_COMPOSE_MAINT", "-DGENOS_COMPOSE_NO_WALK=1"],
    },
    ShaderCompile {
        label: "sun compose",
        compiler: "glslc",
        src: "shaders/lighting_sun_compose.comp",
        flags: &["-DGENOS_COMPOSE_NO_WALK=1"],
    },
    ShaderCompile {
        label: "sun compose hdr",
        compiler: "glslangValidator",
        src: "shaders/lighting_sun_compose.comp",
        flags: &[
            "-V",
            "--target-env",
            "vulkan1.0",
            "-DGENOS_HDR_COMPOSE",
            "-DGENOS_COMPOSE_NO_WALK",
        ],
    },
    ShaderCompile {
        label: "texel detail",
        compiler: "glslc",
        src: "shaders/texel_detail_prod.comp",
        flags: &[],
    },
    ShaderCompile {
        label: "texel detail readonly",
        compiler: "glslc",
        src: "shaders/texel_detail_prod.comp",
        flags: &["-DLIGHTING_SC_WRITES=false", "-DPROD_READONLY=1"],
    },
    ShaderCompile {
        label: "texel detail no walk",
        compiler: "glslc",
        src: "shaders/texel_detail_prod.comp",
        flags: &["-DGENOS_COMPOSE_NO_WALK=1"],
    },
    ShaderCompile {
        label: "direct",
        compiler: "glslc",
        src: "shaders/lighting_direct.comp",
        flags: &[],
    },
    ShaderCompile {
        label: "direct restir",
        compiler: "glslc",
        src: "shaders/lighting_direct.comp",
        flags: &["-DGENOS_RESTIR_DI"],
    },
    ShaderCompile {
        label: "atrous",
        compiler: "glslc",
        src: "shaders/lighting_atrous.comp",
        flags: &[],
    },
    ShaderCompile {
        label: "atrous plain",
        compiler: "glslangValidator",
        src: "shaders/lighting_atrous.comp",
        flags: &[
            "-V",
            "--target-env",
            "vulkan1.0",
            "-DLIGHTING_DENOISE_PLAIN",
        ],
    },
    ShaderCompile {
        label: "restir live",
        compiler: "glslangValidator",
        src: "shaders/restir_live.comp",
        flags: &["-V", "--target-env", "vulkan1.0", "-I../restir-gpu/shaders"],
    },
    ShaderCompile {
        label: "restir spatial",
        compiler: "glslangValidator",
        src: "shaders/restir_live.comp",
        flags: &[
            "-V",
            "--target-env",
            "vulkan1.0",
            "-I../restir-gpu/shaders",
            "-DRESTIR_SPATIAL_ONLY",
            "-DRESTIR_SPATIAL_CAP",
        ],
    },
    ShaderCompile {
        label: "restir spatial wide",
        compiler: "glslangValidator",
        src: "shaders/restir_live.comp",
        flags: &[
            "-V",
            "--target-env",
            "vulkan1.0",
            "-I../restir-gpu/shaders",
            "-DRESTIR_SPATIAL_ONLY",
        ],
    },
];

#[test]
fn production_shader_defines_compile() {
    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    for (index, variant) in PRODUCTION.iter().enumerate() {
        let dest = out_dir.join(format!("shader-define-{index}.spv"));
        let output = Command::new(variant.compiler)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(variant.flags)
            .arg(variant.src)
            .arg("-o")
            .arg(&dest)
            .output()
            .unwrap_or_else(|err| {
                panic!(
                    "{}: failed to run {}: {err}",
                    variant.label, variant.compiler
                )
            });
        assert!(
            output.status.success(),
            "{} ({}) failed to compile {}\n{}",
            variant.label,
            variant.compiler,
            variant.src,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
