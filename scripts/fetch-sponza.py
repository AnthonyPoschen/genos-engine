#!/usr/bin/env python3
"""Download the Crytek Sponza OBJ and Pestana's PBR pack, check sha256, and
write assets/built/sponza as a glTF in metres.

Conversion is not run by the engine build. This script is the reproducible
path: it re-fetches when a file is missing or the checksum does not match,
then rewrites the built glTF. See docs/engine/assets.md.
"""

import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import urllib.request
import zlib

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SOURCE = os.path.join(ROOT, "assets", "source", "sponza")
BUILT = os.path.join(ROOT, "assets", "built", "sponza")

MESH_URL = (
    "https://web.archive.org/web/20140514183901id_/"
    "http://www.crytek.com/download/sponza_obj.rar"
)
MESH_SHA = "0add5d571196781fa3e819821d1b313c5dc0670bb8c89c5705d1cf9c9baa451b"
MESH_NAME = "sponza_obj.rar"

TEX_URL = (
    "https://web.archive.org/web/20170111213656id_/"
    "http://www.alexandre-pestana.com/downloads/SponzaPBR_Textures.rar"
)
TEX_SHA = "4480f9d64adafca57535049ef0c83e5ec74fea33ebfcdcb37b31b0e8b3a99fd7"
TEX_NAME = "SponzaPBR_Textures.rar"

# OBJ centimetres to metres. The world cache texel is 0.125 m.
SCALE = 0.01
DOUBLE_SIDED = {
    "leaf",
    "Material__57",
    "chain",
    "fabric_a",
    "fabric_c",
    "fabric_d",
    "fabric_e",
    "fabric_f",
    "fabric_g",
}


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch(url, path, expect):
    if os.path.isfile(path) and sha256(path) == expect:
        print(f"ok {path}")
        return
    print(f"get {url}")
    tmp = path + ".partial"
    urllib.request.urlretrieve(url, tmp)
    got = sha256(tmp)
    if got != expect:
        os.remove(tmp)
        raise SystemExit(f"{path} sha256 {got} != {expect}")
    os.replace(tmp, path)
    print(f"saved {path} ({os.path.getsize(path)} bytes)")


def extract(archive, dest):
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    os.makedirs(dest)
    subprocess.check_call(["unar", "-quiet", "-output-directory", dest, archive])


def find_one(root, suffix):
    hits = []
    for dirpath, _, names in os.walk(root):
        for name in names:
            if name.lower().endswith(suffix):
                hits.append(os.path.join(dirpath, name))
    if len(hits) != 1:
        raise SystemExit(f"expected one {suffix} under {root}, found {hits}")
    return hits[0]


def case_map(directory):
    found = {}
    for dirpath, _, names in os.walk(directory):
        for name in names:
            found[name.lower()] = os.path.join(dirpath, name)
    return found


def decode_tga(path):
    data = open(path, "rb").read()
    if len(data) < 18:
        raise SystemExit(f"short tga {path}")
    id_len, _cm, image_type, _x, _y, _w0, _h0, _bpp0, _desc0 = struct.unpack_from(
        "<BBBHHBHHB", data, 0
    )
    width, height, bpp, desc = struct.unpack_from("<HHBB", data, 12)
    if image_type not in (2, 3):
        raise SystemExit(f"unsupported tga type {image_type} in {path}")
    start = 18 + id_len
    channels = bpp // 8
    count = width * height * channels
    body = data[start : start + count]
    if len(body) < count:
        raise SystemExit(f"truncated tga {path}")
    rgba = bytearray(width * height * 4)
    if image_type == 3:
        for i, value in enumerate(body):
            rgba[i * 4 : i * 4 + 4] = bytes((value, value, value, 255))
    elif channels == 4:
        for i in range(width * height):
            b, g, r, a = body[i * 4 : i * 4 + 4]
            rgba[i * 4 : i * 4 + 4] = bytes((r, g, b, a))
    elif channels == 3:
        for i in range(width * height):
            b, g, r = body[i * 3 : i * 3 + 3]
            rgba[i * 4 : i * 4 + 4] = bytes((r, g, b, 255))
    else:
        raise SystemExit(f"unsupported bpp {bpp} in {path}")
    if (desc & 0x20) == 0:
        row = width * 4
        flipped = bytearray(len(rgba))
        for y in range(height):
            flipped[y * row : (y + 1) * row] = rgba[(height - 1 - y) * row : (height - y) * row]
        rgba = flipped
    return width, height, bytes(rgba)


def write_png(path, width, height, rgba):
    def chunk(tag, payload):
        return (
            struct.pack(">I", len(payload))
            + tag
            + payload
            + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + rgba[y * width * 4 : (y + 1) * width * 4] for y in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
    with open(path, "wb") as handle:
        handle.write(png)


def alpha_varies(rgba):
    total = len(rgba) // 4
    if total == 0:
        return False
    low = 0
    for i in range(3, len(rgba), 4):
        if rgba[i] < 250:
            low += 1
    return low * 200 > total


def parse_mtl(path, files):
    materials = {}
    current = None
    for line in open(path, encoding="utf-8", errors="replace"):
        parts = line.split()
        if not parts:
            continue
        if parts[0] == "newmtl":
            current = parts[1]
            materials[current] = {"Kd": (0.8, 0.8, 0.8)}
        elif current is None:
            continue
        elif parts[0] in ("Kd", "Ka") and len(parts) >= 4:
            materials[current][parts[0]] = tuple(float(x) for x in parts[1:4])
        elif parts[0] in ("map_Kd", "map_Ka", "map_Ns", "map_NS", "map_d", "map_bump", "bump"):
            name = parts[-1].replace("\\", "/").split("/")[-1]
            materials[current][parts[0]] = files.get(name.lower())
    return materials


def parse_obj(path):
    positions, texcoords, normals = [], [], []
    faces = []
    material = None
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith("v "):
            x, y, z = (float(v) for v in line.split()[1:4])
            positions.append((x * SCALE, y * SCALE, z * SCALE))
        elif line.startswith("vt "):
            u, v = (float(t) for t in line.split()[1:3])
            texcoords.append((u, 1.0 - v))
        elif line.startswith("vn "):
            normals.append(tuple(float(v) for v in line.split()[1:4]))
        elif line.startswith("usemtl "):
            material = line.split()[1]
        elif line.startswith("f "):
            corners = []
            for token in line.split()[1:]:
                bits = token.split("/")
                def index(text, count):
                    if text == "":
                        return None
                    value = int(text)
                    return value - 1 if value > 0 else count + value

                corners.append(
                    (
                        index(bits[0], len(positions)),
                        index(bits[1], len(texcoords)) if len(bits) > 1 else None,
                        index(bits[2], len(normals)) if len(bits) > 2 else None,
                    )
                )
            for i in range(1, len(corners) - 1):
                faces.append((material, (corners[0], corners[i], corners[i + 1])))
    return positions, texcoords, normals, faces


def face_normal(a, b, c):
    ux, uy, uz = b[0] - a[0], b[1] - a[1], b[2] - a[2]
    vx, vy, vz = c[0] - a[0], c[1] - a[1], c[2] - a[2]
    nx, ny, nz = uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx
    length = (nx * nx + ny * ny + nz * nz) ** 0.5
    if length < 1e-12:
        return (0.0, 1.0, 0.0)
    return (nx / length, ny / length, nz / length)


def convert(obj_path, mtl_path, tex_root, out_dir):
    if os.path.isdir(out_dir):
        for name in os.listdir(out_dir):
            path = os.path.join(out_dir, name)
            if name == "README.md":
                continue
            if os.path.isdir(path):
                shutil.rmtree(path)
            else:
                os.remove(path)
    os.makedirs(os.path.join(out_dir, "textures"), exist_ok=True)
    files = case_map(tex_root)
    materials = parse_mtl(mtl_path, files)
    print("parsing obj")
    positions, texcoords, normals, faces = parse_obj(obj_path)
    by_mat = {}
    for material, corners in faces:
        by_mat.setdefault(material, []).append(corners)
    print(f"faces {len(faces)} materials {len(by_mat)}")

    png_of = {}

    def png_for(tga_path):
        if tga_path is None:
            return None
        if tga_path in png_of:
            return png_of[tga_path]
        width, height, rgba = decode_tga(tga_path)
        base = os.path.splitext(os.path.basename(tga_path))[0] + ".png"
        rel = "textures/" + base
        write_png(os.path.join(out_dir, rel), width, height, rgba)
        png_of[tga_path] = (rel, width, height, rgba)
        return png_of[tga_path]

    def channel_png(metal_path, rough_path, name):
        metal = decode_tga(metal_path) if metal_path else None
        rough = decode_tga(rough_path) if rough_path else None
        width, height = (metal or rough)[0], (metal or rough)[1]
        rgba = bytearray(width * height * 4)
        for i in range(width * height):
            # glTF metallic-roughness: G roughness, B metallic.
            g = rough[2][i * 4] if rough else 255
            b = metal[2][i * 4] if metal else 0
            rgba[i * 4 : i * 4 + 4] = bytes((0, g, b, 255))
        rel = f"textures/{name}_metalrough.png"
        write_png(os.path.join(out_dir, rel), width, height, bytes(rgba))
        return rel

    images = []
    image_index = {}

    def add_image(rel):
        if rel in image_index:
            return image_index[rel]
        image_index[rel] = len(images)
        images.append({"uri": rel})
        return image_index[rel]

    buffer = bytearray()
    views = []
    accessors = []

    def align():
        while len(buffer) % 4:
            buffer.append(0)

    def push_bytes(blob, target):
        align()
        offset = len(buffer)
        buffer.extend(blob)
        views.append({"buffer": 0, "byteOffset": offset, "byteLength": len(blob), "target": target})
        return len(views) - 1

    gltf_materials = []
    meshes = []
    nodes = []
    bounds = {}

    for name, tris in by_mat.items():
        info = materials.get(name, {"Kd": (0.6, 0.6, 0.6)})
        albedo = png_for(info.get("map_Kd"))
        normal = png_for(info.get("map_bump"))
        mask = info.get("alphaMode")
        metal = info.get("map_Ka")
        rough = info.get("map_Ns") or info.get("map_NS")
        mr_rel = None
        if metal or rough:
            mr_rel = channel_png(metal, rough, name.replace(" ", "_"))
        alpha_mode = "MASK" if albedo and alpha_varies(albedo[3]) else "OPAQUE"
        used = {}
        pos, nrm, uv, idx = [], [], [], []
        lo = [1e9, 1e9, 1e9]
        hi = [-1e9, -1e9, -1e9]
        for corners in tris:
            fan = []
            for vi, ti, ni in corners:
                key = (vi, ti, ni)
                if key not in used:
                    used[key] = len(pos)
                    p = positions[vi]
                    pos.append(p)
                    for axis in range(3):
                        lo[axis] = min(lo[axis], p[axis])
                        hi[axis] = max(hi[axis], p[axis])
                    if ni is None:
                        nrm.append(None)
                    else:
                        nrm.append(normals[ni])
                    if ti is None:
                        uv.append((0.0, 0.0))
                    else:
                        uv.append(texcoords[ti])
                fan.append(used[key])
            idx.extend(fan)
        if any(n is None for n in nrm):
            # Flat normal per triangle, in the expanded corner order.
            for t in range(0, len(idx), 3):
                n = face_normal(pos[idx[t]], pos[idx[t + 1]], pos[idx[t + 2]])
                for k in range(3):
                    if nrm[idx[t + k]] is None:
                        nrm[idx[t + k]] = n
        nrm = [n if n is not None else (0.0, 1.0, 0.0) for n in nrm]
        bounds[name] = (lo, hi, len(idx) // 3)

        def f32s(values):
            blob = bytearray()
            for value in values:
                for part in value:
                    blob.extend(struct.pack("<f", part))
            return blob

        pos_view = push_bytes(f32s(pos), 34962)
        nrm_view = push_bytes(f32s(nrm), 34962)
        uv_view = push_bytes(f32s(uv), 34962)
        idx_blob = b"".join(struct.pack("<I", i) for i in idx)
        idx_view = push_bytes(idx_blob, 34963)

        def accessor(view, count, kind, ctype, mins=None, maxs=None):
            acc = {
                "bufferView": view,
                "componentType": ctype,
                "count": count,
                "type": kind,
            }
            if mins is not None:
                acc["min"] = mins
                acc["max"] = maxs
            accessors.append(acc)
            return len(accessors) - 1

        attributes = {
            "POSITION": accessor(pos_view, len(pos), "VEC3", 5126, lo, hi),
            "NORMAL": accessor(nrm_view, len(pos), "VEC3", 5126),
            "TEXCOORD_0": accessor(uv_view, len(pos), "VEC2", 5126),
        }
        indices = accessor(idx_view, len(idx), "SCALAR", 5125)
        pbr = {"metallicFactor": 1.0 if mr_rel else 0.0, "roughnessFactor": 1.0}
        if albedo:
            pbr["baseColorTexture"] = {"index": len(gltf_materials)}
        else:
            kd = info.get("Kd", (0.8, 0.8, 0.8))
            pbr["baseColorFactor"] = [kd[0], kd[1], kd[2], 1.0]
        mat = {
            "name": name,
            "pbrMetallicRoughness": pbr,
            "doubleSided": name in DOUBLE_SIDED,
            "alphaMode": alpha_mode,
        }
        if alpha_mode == "MASK":
            mat["alphaCutoff"] = 0.5
        # Textures are appended after we know the image list order. Record slots.
        mat["_albedo"] = albedo[0] if albedo else None
        mat["_normal"] = normal[0] if normal else None
        mat["_mr"] = mr_rel
        gltf_materials.append(mat)
        meshes.append(
            {
                "name": name,
                "primitives": [
                    {
                        "attributes": attributes,
                        "indices": indices,
                        "material": len(meshes),
                    }
                ],
            }
        )
        nodes.append({"name": name, "mesh": len(nodes)})

    textures = []
    for mat in gltf_materials:
        if mat["_albedo"]:
            mat["pbrMetallicRoughness"]["baseColorTexture"] = {
                "index": len(textures)
            }
            textures.append({"source": add_image(mat["_albedo"])})
        if mat["_normal"]:
            mat["normalTexture"] = {"index": len(textures)}
            textures.append({"source": add_image(mat["_normal"])})
        if mat["_mr"]:
            mat["pbrMetallicRoughness"]["metallicRoughnessTexture"] = {
                "index": len(textures)
            }
            textures.append({"source": add_image(mat["_mr"])})
        del mat["_albedo"]
        del mat["_normal"]
        del mat["_mr"]

    gltf = {
        "asset": {"version": "2.0", "generator": "scripts/fetch-sponza.py"},
        "scene": 0,
        "scenes": [{"nodes": list(range(len(nodes)))}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": gltf_materials,
        "textures": textures,
        "images": images,
        "samplers": [{"wrapS": 10497, "wrapT": 10497}],
        "buffers": [{"uri": "sponza.bin", "byteLength": len(buffer)}],
        "bufferViews": views,
        "accessors": accessors,
    }
    with open(os.path.join(out_dir, "sponza.gltf"), "w", encoding="utf-8") as handle:
        json.dump(gltf, handle, separators=(",", ":"))
    with open(os.path.join(out_dir, "sponza.bin"), "wb") as handle:
        handle.write(buffer)
    print("bounds (metres)")
    for name, (lo, hi, tris) in sorted(bounds.items()):
        size = [hi[a] - lo[a] for a in range(3)]
        print(f"  {name}: tris {tris} min {lo} max {hi} size {size}")
    return bounds


def directory_bytes(path):
    total = 0
    for dirpath, _, names in os.walk(path):
        for name in names:
            total += os.path.getsize(os.path.join(dirpath, name))
    return total


def main():
    os.makedirs(SOURCE, exist_ok=True)
    fetch(MESH_URL, os.path.join(SOURCE, MESH_NAME), MESH_SHA)
    fetch(TEX_URL, os.path.join(SOURCE, TEX_NAME), TEX_SHA)
    stamp = os.path.join(BUILT, "SOURCE.sha256")
    want = f"{MESH_SHA}  {MESH_NAME}\n{TEX_SHA}  {TEX_NAME}\n"
    gltf = os.path.join(BUILT, "sponza.gltf")
    if os.path.isfile(stamp) and open(stamp).read() == want and os.path.isfile(gltf) and "--force" not in sys.argv:
        print("built sponza matches the pinned checksums")
    else:
        with tempfile.TemporaryDirectory(prefix="sponza-") as tmp:
            extract(os.path.join(SOURCE, MESH_NAME), os.path.join(tmp, "mesh"))
            extract(os.path.join(SOURCE, TEX_NAME), os.path.join(tmp, "tex"))
            obj = find_one(os.path.join(tmp, "mesh"), ".obj")
            tex_dir = os.path.dirname(find_one(os.path.join(tmp, "tex"), ".mtl"))
            mtl = os.path.join(tex_dir, "sponzaPBR.mtl")
            if not os.path.isfile(mtl):
                mtl = find_one(os.path.join(tmp, "tex"), ".mtl")
            copyright_src = os.path.join(tex_dir, "copyright.txt")
            if os.path.isfile(copyright_src):
                shutil.copyfile(copyright_src, os.path.join(SOURCE, "copyright.txt"))
            convert(obj, mtl, tex_dir, BUILT)
        with open(stamp, "w", encoding="utf-8") as handle:
            handle.write(want)
        readme = os.path.join(BUILT, "README.md")
        with open(readme, "w", encoding="utf-8") as handle:
            handle.write(
                "Engine-ready Sponza in metres. Rebuild with `scripts/fetch-sponza.py`.\n"
                "Positions are the Crytek OBJ centimetres times 0.01.\n"
                "The renderer has no specular lobe: it treats base colour times (1 - metallic)\n"
                "as the diffuse albedo and keeps roughness on the material.\n"
            )
    source_bytes = directory_bytes(SOURCE)
    built_bytes = directory_bytes(BUILT)
    print(f"LFS source bytes {source_bytes}")
    print(f"LFS built bytes {built_bytes}")
    print(f"LFS total bytes {source_bytes + built_bytes}")
    print("text credits are not included in the LFS total above if they are CREDITS.md or copyright.txt")
    lfs = 0
    for folder in (SOURCE, BUILT):
        for dirpath, _, names in os.walk(folder):
            for name in names:
                if name.endswith((".md", ".txt", ".sha256")):
                    continue
                lfs += os.path.getsize(os.path.join(dirpath, name))
    print(f"binary bytes to store in LFS {lfs}")


if __name__ == "__main__":
    main()
