//! OBJ + MTL scenes and FBX (binary and ASCII) through the glTF scene shape.

use genos_load::{load_fbx, load_obj_scene, AlphaMode, LoadError, MemorySource};

fn no_files(_: &str) -> Result<Vec<u8>, LoadError> {
    Err(LoadError::Unrecognized)
}

const OBJ: &str = "# two quads, two materials\n\
mtllib scene.mtl\n\
o thing\n\
v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\n\
vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
vn 0 0 1\n\
g front\ns 1\nusemtl red\n\
f 1/1/1 2/2/1 3/3/1 4/4/1\n\
usemtl glass\n\
f -6 -5 -1 -2\n";

const MTL: &str = "newmtl red\nKd 0.8 0.1 0.1\nNs 48\nmap_Kd missing.png\n\
newmtl glass\nKd 0.9 0.9 0.9\nd 0.25\nKe 0 0 0\n";

#[test]
fn obj_scene_groups_by_material_and_reads_mtl() {
    let resolve = |name: &str| -> Result<Vec<u8>, LoadError> {
        if name == "scene.mtl" {
            Ok(MTL.as_bytes().to_vec())
        } else {
            Err(LoadError::Unrecognized)
        }
    };
    let scene = load_obj_scene(&MemorySource::new(OBJ.as_bytes()), &resolve).unwrap();
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.instances.len(), 1);
    let prims = &scene.meshes[0].primitives;
    assert_eq!(prims.len(), 2);
    // Quad 1: welded to 4 vertices, two triangles, smooth normals, flipped v.
    assert_eq!(prims[0].positions.len(), 4);
    assert_eq!(prims[0].indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(prims[0].normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(prims[0].texcoords[0], [0.0, 1.0]);
    // Quad 2 (negative indices, no normals): flat normal of 0 0 0, 1 0 0, 1 0 1, 0 0 1.
    assert_eq!(prims[1].indices.len(), 6);
    assert!(prims[1].texcoords.is_empty());
    let n = prims[1].normals[0];
    assert!((n[1].abs() - 1.0).abs() < 1e-6, "{n:?}");
    let red = &scene.materials[prims[0].material.unwrap()];
    assert_eq!(red.name.as_deref(), Some("red"));
    assert_eq!(&red.base_color[..3], &[0.8, 0.1, 0.1]);
    assert!((red.roughness - 0.2).abs() < 1e-6);
    // The missing texture is left off.
    assert!(red.base_color_texture.is_none());
    assert_eq!(red.metallic, 0.0);
    let glass = &scene.materials[prims[1].material.unwrap()];
    assert_eq!(glass.base_color[3], 0.25);
    assert_eq!(glass.alpha_mode, AlphaMode::Blend);
}

#[test]
fn obj_scene_without_mtl_uses_default_material() {
    let scene = load_obj_scene(&MemorySource::new(OBJ.as_bytes()), &no_files).unwrap();
    assert!(scene.materials.is_empty());
    assert!(scene.meshes[0].primitives.iter().all(|p| p.material.is_none()));
}

#[test]
fn obj_scene_rejects_bad_indices() {
    let bad = "v 0 0 0\nv 1 0 0\nf 1 2 3\n";
    assert!(load_obj_scene(&MemorySource::new(bad.as_bytes()), &no_files).is_err());
}

// A minimal binary FBX writer (7.4 headers, 32-bit offsets; 7.5, 64-bit).
enum P {
    I(i32),
    L(i64),
    D(f64),
    S(&'static str),
    Ad(Vec<f64>),
    Ai(Vec<i32>),
}
struct N {
    name: &'static str,
    props: Vec<P>,
    kids: Vec<N>,
}
fn n(name: &'static str, props: Vec<P>, kids: Vec<N>) -> N {
    N { name, props, kids }
}
fn write_node(out: &mut Vec<u8>, node: &N, wide: bool) {
    let word = |out: &mut Vec<u8>, at: usize, v: u64| {
        if wide {
            out[at..at + 8].copy_from_slice(&v.to_le_bytes());
        } else {
            out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
        }
    };
    let w = if wide { 8 } else { 4 };
    let head = out.len();
    out.resize(head + 3 * w, 0);
    out.push(node.name.len() as u8);
    out.extend_from_slice(node.name.as_bytes());
    let props_at = out.len();
    for p in &node.props {
        match p {
            P::I(v) => {
                out.push(b'I');
                out.extend_from_slice(&v.to_le_bytes());
            }
            P::L(v) => {
                out.push(b'L');
                out.extend_from_slice(&v.to_le_bytes());
            }
            P::D(v) => {
                out.push(b'D');
                out.extend_from_slice(&v.to_le_bytes());
            }
            P::S(s) => {
                out.push(b'S');
                out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            P::Ad(v) => {
                out.push(b'd');
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&((v.len() * 8) as u32).to_le_bytes());
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            P::Ai(v) => {
                out.push(b'i');
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&((v.len() * 4) as u32).to_le_bytes());
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
    }
    let props_len = out.len() - props_at;
    for k in &node.kids {
        write_node(out, k, wide);
    }
    if !node.kids.is_empty() {
        out.resize(out.len() + 3 * w + 1, 0);
    }
    let end = out.len() as u64;
    word(out, head, end);
    word(out, head + w, node.props.len() as u64);
    word(out, head + 2 * w, props_len as u64);
}
fn p70(entries: Vec<N>) -> N {
    n("Properties70", vec![], entries)
}
fn pv(name: &'static str, v: [f64; 3]) -> N {
    n(
        "P",
        vec![P::S(name), P::S(name), P::S(""), P::S("A"), P::D(v[0]), P::D(v[1]), P::D(v[2])],
        vec![],
    )
}
fn pi(name: &'static str, v: i32) -> N {
    n("P", vec![P::S(name), P::S("int"), P::S("Integer"), P::S(""), P::I(v)], vec![])
}
fn pd(name: &'static str, v: f64) -> N {
    n("P", vec![P::S(name), P::S("double"), P::S("Number"), P::S(""), P::D(v)], vec![])
}

fn fbx_file(version: u32) -> Vec<u8> {
    let wide = version >= 7500;
    let tree = vec![
        n(
            "GlobalSettings",
            vec![],
            vec![p70(vec![
                pi("UpAxis", 2),
                pi("UpAxisSign", 1),
                pi("FrontAxis", 1),
                pi("FrontAxisSign", -1),
                pi("CoordAxis", 0),
                pi("CoordAxisSign", 1),
                pd("UnitScaleFactor", 100.0),
            ])],
        ),
        n(
            "Objects",
            vec![],
            vec![
                n(
                    "Geometry",
                    vec![P::L(10), P::S("Quad\0\x01Geometry"), P::S("Mesh")],
                    vec![
                        n("Vertices", vec![P::Ad(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.])], vec![]),
                        n("PolygonVertexIndex", vec![P::Ai(vec![0, 1, 2, !3])], vec![]),
                        n(
                            "LayerElementNormal",
                            vec![P::I(0)],
                            vec![
                                n("MappingInformationType", vec![P::S("ByVertice")], vec![]),
                                n("ReferenceInformationType", vec![P::S("Direct")], vec![]),
                                n("Normals", vec![P::Ad([0., 0., 1.].repeat(4))], vec![]),
                            ],
                        ),
                        n(
                            "LayerElementUV",
                            vec![P::I(0)],
                            vec![
                                n("MappingInformationType", vec![P::S("ByPolygonVertex")], vec![]),
                                n("ReferenceInformationType", vec![P::S("IndexToDirect")], vec![]),
                                n("UV", vec![P::Ad(vec![0., 0., 1., 0., 1., 1., 0., 1.])], vec![]),
                                n("UVIndex", vec![P::Ai(vec![0, 1, 2, 3])], vec![]),
                            ],
                        ),
                        n(
                            "LayerElementMaterial",
                            vec![P::I(0)],
                            vec![
                                n("MappingInformationType", vec![P::S("AllSame")], vec![]),
                                n("Materials", vec![P::Ai(vec![0])], vec![]),
                            ],
                        ),
                    ],
                ),
                n(
                    "Model",
                    vec![P::L(20), P::S("Parent\0\x01Model"), P::S("Null")],
                    vec![p70(vec![pv("Lcl Translation", [5.0, 0.0, 0.0])])],
                ),
                n(
                    "Model",
                    vec![P::L(21), P::S("Quad\0\x01Model"), P::S("Mesh")],
                    vec![p70(vec![
                        pv("Lcl Rotation", [0.0, 0.0, 90.0]),
                        pv("Lcl Scaling", [2.0, 2.0, 2.0]),
                    ])],
                ),
                n(
                    "Material",
                    vec![P::L(30), P::S("Paint\0\x01Material"), P::S("")],
                    vec![p70(vec![pv("DiffuseColor", [0.2, 0.4, 0.6]), pd("Opacity", 1.0)])],
                ),
            ],
        ),
        n(
            "Connections",
            vec![],
            vec![
                n("C", vec![P::S("OO"), P::L(20), P::L(0)], vec![]),
                n("C", vec![P::S("OO"), P::L(21), P::L(20)], vec![]),
                n("C", vec![P::S("OO"), P::L(10), P::L(21)], vec![]),
                n("C", vec![P::S("OO"), P::L(30), P::L(21)], vec![]),
            ],
        ),
    ];
    let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    out.extend_from_slice(&version.to_le_bytes());
    for node in &tree {
        write_node(&mut out, node, wide);
    }
    out.resize(out.len() + if wide { 25 } else { 13 }, 0);
    out
}

fn check_fbx_quad(scene: &genos_load::GltfScene) {
    assert_eq!(scene.meshes.len(), 1);
    let prim = &scene.meshes[0].primitives[0];
    assert_eq!(prim.positions.len(), 4);
    assert_eq!(prim.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(prim.texcoords[2], [1.0, 0.0]);
    let m = &scene.materials[prim.material.unwrap()];
    assert_eq!(m.name.as_deref(), Some("Paint"));
    assert!((m.base_color[2] - 0.6).abs() < 1e-6);
    // World: Z up -> Y up, metres (unit 100 cm), parent +5 x, rotate 90 about
    // z, scale 2. Vertex (1, 0, 0): scale (2, 0, 0), rotate (0, 2, 0), +5 x
    // (5, 2, 0); Z up -> Y up: (x, z, -y) = (5, 0, -2).
    let w = genos_load::transform_point(&scene.instances[0].transform, prim.positions[1]);
    for (a, b) in w.iter().zip([5.0, 0.0, -2.0]) {
        assert!((a - b).abs() < 1e-5, "{w:?}");
    }
}

#[test]
fn binary_fbx_7400_and_7500() {
    for version in [7400, 7500] {
        let bytes = fbx_file(version);
        let scene = load_fbx(&MemorySource::new(&bytes), &no_files).unwrap();
        check_fbx_quad(&scene);
    }
}

#[test]
fn binary_fbx_truncated_is_an_error() {
    let bytes = fbx_file(7400);
    let cut = &bytes[..bytes.len() / 2];
    assert!(load_fbx(&MemorySource::new(cut), &no_files).is_err());
}

const ASCII: &str = "; FBX 7.4.0 project file\n\
FBXHeaderExtension:  {\n\tFBXVersion: 7400\n}\n\
GlobalSettings:  {\n\tProperties70:  {\n\
\t\tP: \"UpAxis\", \"int\", \"Integer\", \"\",2\n\
\t\tP: \"FrontAxis\", \"int\", \"Integer\", \"\",1\n\
\t\tP: \"FrontAxisSign\", \"int\", \"Integer\", \"\",-1\n\
\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",100\n\t}\n}\n\
Objects:  {\n\
\tGeometry: 10, \"Geometry::Quad\", \"Mesh\" {\n\
\t\tVertices: *12 {\n\t\t\ta: 0,0,0,1,0,0,\n1,1,0,0,1,0\n\t\t}\n\
\t\tPolygonVertexIndex: *4 {\n\t\t\ta: 0,1,2,-4\n\t\t}\n\
\t\tLayerElementNormal: 0 {\n\
\t\t\tMappingInformationType: \"ByVertice\"\n\
\t\t\tReferenceInformationType: \"Direct\"\n\
\t\t\tNormals: *12 {\n\t\t\t\ta: 0,0,1,0,0,1,0,0,1,0,0,1\n\t\t\t}\n\t\t}\n\
\t\tLayerElementUV: 0 {\n\
\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\
\t\t\tReferenceInformationType: \"IndexToDirect\"\n\
\t\t\tUV: *8 {\n\t\t\t\ta: 0,0,1,0,1,1,0,1\n\t\t\t}\n\
\t\t\tUVIndex: *4 {\n\t\t\t\ta: 0,1,2,3\n\t\t\t}\n\t\t}\n\
\t\tLayerElementMaterial: 0 {\n\
\t\t\tMappingInformationType: \"AllSame\"\n\
\t\t\tMaterials: *1 {\n\t\t\t\ta: 0\n\t\t\t}\n\t\t}\n\t}\n\
\tModel: 20, \"Model::Parent\", \"Null\" {\n\t\tProperties70:  {\n\
\t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",5,0,0\n\t\t}\n\t}\n\
\tModel: 21, \"Model::Quad\", \"Mesh\" {\n\t\tProperties70:  {\n\
\t\t\tP: \"Lcl Rotation\", \"Lcl Rotation\", \"\", \"A\",0,0,90\n\
\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",2,2,2\n\t\t}\n\t}\n\
\tMaterial: 30, \"Material::Paint\", \"\" {\n\t\tProperties70:  {\n\
\t\t\tP: \"DiffuseColor\", \"Color\", \"\", \"A\",0.2,0.4,0.6\n\t\t}\n\t}\n}\n\
; comment\n\
Connections:  {\n\
\tC: \"OO\",20,0\n\tC: \"OO\",21,20\n\tC: \"OO\",10,21\n\tC: \"OO\",30,21\n}\n";

#[test]
fn ascii_fbx_matches_binary() {
    let scene = load_fbx(&MemorySource::new(ASCII.as_bytes()), &no_files).unwrap();
    check_fbx_quad(&scene);
}

#[test]
fn model_file_dispatch() {
    let dir = std::env::temp_dir().join(format!("genos-load-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.obj"), OBJ).unwrap();
    std::fs::write(dir.join("scene.mtl"), MTL).unwrap();
    std::fs::write(dir.join("b.fbx"), fbx_file(7500)).unwrap();
    let obj = genos_load::load_model_file(dir.join("a.obj")).unwrap();
    assert_eq!(obj.materials.len(), 2);
    let fbx = genos_load::load_model_file(dir.join("b.fbx")).unwrap();
    check_fbx_quad(&fbx);
    let gltf = genos_load::load_model_file(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/quad.glb")).unwrap();
    assert!(!gltf.meshes.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}
