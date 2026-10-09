"""Writes the glTF test fixtures (run from this directory). Needs only the standard library."""
import base64, json, struct

def pad4(b, fill=b'\0'):
    return b + fill * (-len(b) % 4)

# 1. cube.gltf: one buffer as a data: URI, u16 indices, normals, a parent/child TRS chain.
P = [(-1,-1,-1),(1,-1,-1),(1,1,-1),(-1,1,-1),(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]
I = [0,2,1, 0,3,2, 4,5,6, 4,6,7, 0,1,5, 0,5,4, 3,7,6, 3,6,2, 0,4,7, 0,7,3, 1,2,6, 1,6,5]
pos = b''.join(struct.pack('<3f', *p) for p in P)
nrm = b''.join(struct.pack('<3f', *[c / 3**0.5 for c in p]) for p in P)
idx = pad4(struct.pack('<36H', *I))
buf = pos + nrm + idx
cube = {
  "asset": {"version": "2.0"},
  "scene": 0,
  "scenes": [{"nodes": [0]}],
  "nodes": [
    {"name": "parent", "translation": [10, 0, 0], "children": [1]},
    {"name": "child", "mesh": 0, "rotation": [0, 0.7071067811865476, 0, 0.7071067811865476], "scale": [2, 2, 2]},
  ],
  "meshes": [{"name": "cube", "primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0}]}],
  "materials": [{"name": "red", "pbrMetallicRoughness": {"baseColorFactor": [0.8, 0.1, 0.1, 1], "metallicFactor": 0.25, "roughnessFactor": 0.5},
                 "alphaMode": "MASK", "alphaCutoff": 0.3, "doubleSided": True}],
  "buffers": [{"byteLength": len(buf), "uri": "data:application/octet-stream;base64," + base64.b64encode(buf).decode()}],
  "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 96}, {"buffer": 0, "byteOffset": 96, "byteLength": 96},
                  {"buffer": 0, "byteOffset": 192, "byteLength": 72}],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3", "min": [-1,-1,-1], "max": [1,1,1]},
    {"bufferView": 1, "componentType": 5126, "count": 8, "type": "VEC3"},
    {"bufferView": 2, "componentType": 5123, "count": 36, "type": "SCALAR"},
  ],
}
open('cube.gltf', 'w').write(json.dumps(cube, indent=1))

# 2. quad.glb: interleaved position+uv (byteStride 20), u8 indices, no normals (flat),
# a triangle strip, a JPEG base colour in a bufferView, emissive strength extension.
jpg = open('gradient-444.jpg', 'rb').read()
inter = b''.join(struct.pack('<5f', x, y, 0, u, v) for (x, y, u, v) in [(0,0,0,1),(1,0,1,1),(0,1,0,0),(1,1,1,0)])
idx8 = pad4(bytes([0, 1, 2, 2, 1, 3]))
strip = pad4(bytes([0, 1, 2, 3]))
img_off = len(inter) + len(idx8) + len(strip)
binc = inter + idx8 + strip + pad4(jpg)
quad = {
  "asset": {"version": "2.0", "generator": "make_gltf.py"},
  "extensionsUsed": ["KHR_materials_emissive_strength"],
  "extensionsRequired": ["KHR_materials_emissive_strength"],
  "nodes": [{"mesh": 0, "matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,-5,1]}],
  "meshes": [{"primitives": [
      {"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "indices": 2, "material": 0},
      {"attributes": {"POSITION": 0}, "indices": 3, "mode": 5},
      {"attributes": {"POSITION": 0}, "mode": 1}]}],
  "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}},
                 "emissiveFactor": [1, 0.5, 0.25], "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": 4}}}],
  "textures": [{"source": 0, "sampler": 0}],
  "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 33071}],
  "images": [{"bufferView": 3, "mimeType": "image/jpeg"}],
  "buffers": [{"byteLength": len(binc)}],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": len(inter), "byteStride": 20},
    {"buffer": 0, "byteOffset": len(inter), "byteLength": 6},
    {"buffer": 0, "byteOffset": len(inter) + len(idx8), "byteLength": 4},
    {"buffer": 0, "byteOffset": img_off, "byteLength": len(jpg)},
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
    {"bufferView": 0, "byteOffset": 12, "componentType": 5126, "count": 4, "type": "VEC2"},
    {"bufferView": 1, "componentType": 5121, "count": 6, "type": "SCALAR"},
    {"bufferView": 2, "componentType": 5121, "count": 4, "type": "SCALAR"},
  ],
}
js = pad4(json.dumps(quad).encode(), b' ')
glb = struct.pack('<III', 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(binc))
glb += struct.pack('<II', len(js), 0x4E4F534A) + js + struct.pack('<II', len(binc), 0x004E4942) + binc
open('quad.glb', 'wb').write(glb)

# 3. tri.gltf: external tri.bin and quad.png; a sparse accessor moves one vertex; no scene list.
tri_pos = struct.pack('<9f', 0,0,0, 1,0,0, 0,1,0)
sp_idx = pad4(struct.pack('<H', 2))
sp_val = struct.pack('<3f', 0, 3, 0)
tbin = tri_pos + sp_idx + sp_val
open('tri.bin', 'wb').write(tbin)
tri = {
  "asset": {"version": "2.0"},
  "nodes": [{"mesh": 0, "translation": [0, 1, 0]}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0}]}],
  "materials": [{"emissiveTexture": {"index": 0, "texCoord": 0}, "normalTexture": {"index": 0, "scale": 0.5}}],
  "textures": [{"source": 0}],
  "images": [{"uri": "quad.png"}],
  "buffers": [{"byteLength": len(tbin), "uri": "tri.bin"}],
  "bufferViews": [{"buffer": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 2}, {"buffer": 0, "byteOffset": 40, "byteLength": 12}],
  "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                 "sparse": {"count": 1, "indices": {"bufferView": 1, "componentType": 5123}, "values": {"bufferView": 2}}}],
}
open('tri.gltf', 'w').write(json.dumps(tri, indent=1))
