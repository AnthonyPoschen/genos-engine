# Sponza

Engine-ready files are `assets/built/sponza/`. They are not regenerated on every build: the OBJ is about 22 MB and the textures are TGA, so `scripts/fetch-sponza.py` does the conversion once and writes the glTF. The built positions are metres (OBJ centimetres times 0.01). The world-cache texel is 0.125 m, which matches a building of this size and does not match the raw centimetre coordinates.

## Mesh

- Work: Crytek Sponza, OBJ release `sponza_obj.rar` (20 Aug 2010 header in the file)
- Authors: Frank Meinl, 2010. The original atrium is Marko Dabrovic, early 2002.
- URL: `https://web.archive.org/web/20140514183901id_/http://www.crytek.com/download/sponza_obj.rar`
- Original page: `http://www.crytek.com/download/sponza_obj.rar` (dead; the Wayback id above is the file)
- sha256: `0add5d571196781fa3e819821d1b313c5dc0670bb8c89c5705d1cf9c9baa451b`
- Bytes: 3875156
- Licence: Crytek donated this model to the public. The note in the texture pack says "donated to the public". There is no CC name on the archive.

## Textures

- Work: PBR textures for the Sponza model, `SponzaPBR_Textures.rar`
- Author: Alexandre Pestana
- Page: `http://www.alexandre-pestana.com/pbr-textures-sponza/`
- URL: `https://web.archive.org/web/20170111213656id_/http://www.alexandre-pestana.com/downloads/SponzaPBR_Textures.rar`
- sha256: `4480f9d64adafca57535049ef0c83e5ec74fea33ebfcdcb37b31b0e8b3a99fd7`
- Bytes: 100370565
- Licence: Pestana wrote that he does not put any restriction on the use of those new textures. The OBJ materials and the original textures are Frank Meinl's, with later edits by Morgan McGuire (14 Jul 2011). The pack's `copyright.txt` is reproduced below and must stay with the textures.

The RAR does not contain `sponza_thorn_mask.tga`, `vase_plant_mask.tga`, or `chain_texture_mask.tga`. Thorn and the vase plant already carry a mask in the diffuse alpha (McGuire's note). Chain albedo alpha is opaque, so the chains stay opaque. Curtain and fabric albedo alpha is opaque in this pack; those sheets are still double-sided. `Background_roughness.tga` in the MTL is `Background_Roughness.tga` on disk.

## copyright.txt

```text
PBR textures for the Sponza model.
For more informations: www.alexandre-pestana.com


Original copyright:

July 14, 2011 Morgan McGuire modified the model from Crytek's OBJ
export to correct some small errors.  He computed bump maps from the
normal maps using <a
href="http://cs.williams.edu/~morgan/code/">normal2bump.cpp</a> (since
MTL files expect height bumps, not normals), put the "mask" textures
into the alpha channel of the associated diffuse texture, cleaned up
noise in the masks, created the missing gi_flag.tga texture, and
removed the long untextured banner floating in the middle of the
atrium that appears in the file but in none of the published images of
the model.  The banner is in banner.obj.



http://www.crytek.com/cryengine/cryengine3/downloads


Sponza Model
August 19, 2010
The Atrium Sponza Palace, Dubrovnik, is an elegant and improved model created by Frank Meinl. The original Sponza model was created by Marko Dabrovic in early 2002. Over the years, the Sponza Atrium scene has become one of the most popular 3D scenes for testing global illumination and radiosity due to it's specific architectural structure which is particularly complex for global illumination light.

However, nowadays it is considered as a simple model, thus it was decided to crate a new model with highly improved appearance and scene complexity. It is donated to the public for  radiosity and is represented in several different formats (3ds, Obj) for use with various commercial 3D applications and renderers.


Screenshot from the I3D paper
http://crytek.com/sites/default/files/20100301_lpv.pdf
```
