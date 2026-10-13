# Art assets

Art that the engine draws lives under `assets/`. Git LFS stores the bytes. Git stores the pointers, the credits, and this note.

## Layout

```text
assets/source/<name>/    untouched originals, plus a credits file
assets/built/<name>/     engine-ready files, when conversion is not cheap
```

`assets/source/<name>/` is the archive or the files as the author published them. Do not edit those bytes. Put a `CREDITS.md` next to them with the source URL, the checksum, the licence, and who to name.

`assets/built/<name>/` is what the engine loads. Sponza is stored here as a glTF in metres (positions from the OBJ centimetres, times 0.01), with PNG textures. Parsing the 22 MB OBJ and decoding the TGA pack is not cheap enough to repeat on every build, so the built files are in LFS. `scripts/fetch-sponza.py` is the way to pull the originals again, check the checksums, and rewrite the built files.

A conversion that is one small decode with no third-party archive tool can run at build time instead of landing in `assets/built/`. Say which choice a new asset uses in its `CREDITS.md`.

## Adding an asset

1. Read the licence. If the terms are missing, or they forbid redistribution, stop. Do not commit the bytes.
2. `git lfs install` once on the machine (below).
3. Put the originals in `assets/source/<name>/` and write `CREDITS.md` before the first commit of those files.
4. Either commit the engine-ready files under `assets/built/<name>/`, or generate them at build time and say so in the credits.
5. Keep a fetch script when the originals come from a URL. The script downloads only when the file is missing or the checksum does not match, then checks the checksum again.
6. Commit the files. Git stores the pointers. Push the branch normally so Git LFS uploads the objects. This account is pay-as-you-go and has no quota.

## Licence and attribution

Every asset names:

- the work, the authors, and the year
- the exact URL the bytes came from
- the sha256 of each archive
- the licence, in the author's words where the author did not use a standard name
- what must ship with the files (a copyright note, an unchanged original)

Sponza is the example: `assets/source/sponza/CREDITS.md`. The mesh is the Crytek 2010 OBJ (Frank Meinl; the 2002 model is Marko Dabrovic's), donated to the public. The PBR textures are Alexandre Pestana's pack. Morgan McGuire's 2011 note is part of that pack and stays in the credits. This tree does not use Unity Classic Sponza or any derivative of it.

## Git LFS

Install the filters once:

```sh
git lfs install
```

`.gitattributes` tracks the binary types under `assets/` (glTF and `.bin`, `.glb`, `.obj`, `.fbx`, images including `.png`, `.jpg`, `.tga`, `.dds`, `.ktx2`, `.exr`, `.hdr`, audio including `.wav` and `.ogg`, and the source archives). `CREDITS.md` and other text stay in ordinary git.

A clone that smudges LFS replaces each pointer with the bytes. A clone that skips smudge leaves the pointer text in the working tree. The engine cannot load a pointer.

## CI and cloud agents

A job that does not draw the textured scenes does not download the LFS bytes. That is the default.

`.lfsconfig` sets `lfs.fetchexclude` to `assets/built/**,assets/source/**`. A normal clone and a plain `git lfs pull` leave those paths as pointer text. Push is not affected: the objects still upload with the branch.

`GIT_LFS_SKIP_SMUDGE=1` is the same skip for a checkout that does not read `.lfsconfig` yet:

```sh
GIT_LFS_SKIP_SMUDGE=1 git clone <url>
# or, in an existing clone:
git lfs install --skip-smudge
```

Pull one asset when a job does need it. `--exclude=` clears the default so the include is not filtered back out:

```sh
git lfs pull --include="assets/built/sponza/**,assets/source/sponza/**" --exclude=""
```

`lfs.fetchexclude` does not delete pointers already in the working tree.

Do not point a checkout at `$HOME` to store these files. The lighting debug binary writes its own screenshots under `./genos-debug/`, which is gitignored and is not an LFS path.
