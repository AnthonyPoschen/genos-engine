# The first proof is a camera over shapes

[ADR 0010](0010-first-proof-is-the-lit-scripted-scene.md) replaces this proof. The text below is the earlier cut.

The first program opens one window and draws shapes with Vulkan. The keyboard and the mouse move the camera. The picture and the motion prove the window, the renderer, and the controls together. The editor comes after this proof. Gamepad support lands in the input port. The camera proof does not require a gamepad.

**Rejected:** An editor shell before a rendered scene. A triangle with no camera.
