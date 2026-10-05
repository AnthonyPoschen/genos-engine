# Lighting uses radiance cascades

Genos Engine lighting uses radiance cascades. Alexander Sannikov presented that structure for Path of Exile 2. The penumbra hypothesis says a near emitter needs more positions, and a far emitter needs more directions. Each cascade stores one distance range of the radiance field. The field is built again every frame. The build does not reuse the previous frame.

The engine lights a 3D scene from a moving camera. The cascades are anchored in the world, not to the camera. A nearer cascade keeps more positions and fewer directions. A farther cascade does the reverse. A world probe is a coarse probe past those ranges. It samples material-colored light the nearer ranges miss, including light from outside the floor, and that sample fills an empty ray. The first cascade picture is the scene in [ADR 0010](0010-first-proof-is-the-lit-scripted-scene.md).

The working notes, the paper links, and the merge rule are in [Lighting](../lighting.md).

- Paper repository: <https://github.com/Raikiri/RadianceCascadesPaper>
- PDF: <https://github.com/Raikiri/RadianceCascadesPaper/blob/main/out_latexmk2/RadianceCascades.pdf>
- Overview: <https://80.lv/articles/radiance-cascades-new-approach-to-calculating-global-illumination>

The cascade code calls Vulkan. See [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md).

**Rejected:** Baked lighting as the primary field. A lighting middleware crate. A rule that every emitter stays inside the camera view.
