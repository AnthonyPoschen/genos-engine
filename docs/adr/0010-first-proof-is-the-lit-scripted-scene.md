# The first proof is the lit scripted scene

[ADR 0005](0005-first-proof-is-a-camera-over-shapes.md) stopped at an unlit camera. The first proof is now one Vulkan window, one character-controller input map, one Rhai scene, and one radiance-cascade picture. The done checks are in `docs/goals/mvp-scene.md`.

The camera moves on the ground plane. `W`, `A`, `S`, `D`, and the left stick move it. Mouse motion and the right stick turn it. The script places the white floor, the corner walls, the colored solids, and the lights. The picture shows bounced color on a white surface.

**Rejected:** An unlit triangle as the done picture. A physics body for the camera. A world probe as a requirement of this proof.
