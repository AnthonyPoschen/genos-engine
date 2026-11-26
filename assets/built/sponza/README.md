Engine-ready Sponza in metres. Rebuild with `scripts/fetch-sponza.py`.
Positions are the Crytek OBJ centimetres times 0.01.
The renderer's old path has no specular lobe: it treats base colour times
(1 - metallic) as the diffuse albedo and keeps roughness on the material, so
the metal (flagpoles, hanging vases, chains, the curtains' gold thread) is
black there. Cone mode draws the metal's specular light
(`cone_cascades.specular`).
