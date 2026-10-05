# Breaking Point

Breaking Point is the 2013 C++ engine at `https://github.com/AnthonyPoschen/Breaking-Point`. Read it for ideas. Write the Genos Engine math yourself. Write the Vulkan camera yourself.

This repository has no checkout of Breaking Point. Open the GitHub URL. The useful code is on the `master` branch.

## Read it for these ideas

- `src/Math/float2.h`, `src/Math/float3.h`, and `src/Math/float4.h` show length, dot, cross, unit length, and lerp.
- `src/Math/matrix4.h` and `src/Math/matrix4.cpp` show identity, scale, rotate, translate, and matrix multiply.
- `src/Math/Quaternion.h` and `src/Math/Quaternion.cpp` show normalize, slerp, axis-angle, and a matrix conversion.
- Slerp in `src/Math/Quaternion.cpp` flips one side when the dot product is negative, to take the short arc.
- `src/Core/Camera.cpp` builds a view basis from the camera position, a look target, and an up vector.
- The default up vector is `(0, 1, 0)`.
- The camera stores a field of view, an aspect ratio, a near plane, and a far plane.
- `Update` builds the view matrix, the projection matrix, and the view-projection matrix.
- `src/Core/Transform.h` stores translation, rotation, and scale.
- `DrawScene` in `src/Graphics/Renderer.cpp` combines the model, the view, and the projection into one matrix.

## Leave these behind

Breaking Point was a working engine, and it has no lighting pass. Use the radiance cascade plan for lighting. See [ADR 0008](../adr/0008-lighting-uses-radiance-cascades.md).

Breaking Point embedded Python 3.2. Scripts in Genos Engine use Rhai. See [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md).

The window and the renderer use OpenGL 4.2, WGL, and Win32. Vulkan and the current operating systems replace that path. See [ADR 0003](../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md) and [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md).

The projection numbers in `Camera.cpp` belong to that OpenGL path. Vulkan clip space is different. Derive the projection for Vulkan. Check that projection with `examples/camera`.

`Renderer.cpp` transposes the translation matrix before use. Treat that transpose as a storage detail of this old code. Choose one multiply order. Test that order with the camera proof.

Skip `src/3rd Party Content/` and `Release/`. Those trees vendor Boost, GLEW, and Python 3.2.
