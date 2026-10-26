// Apply one froxel column: background * transmittance + in-scattering.
// scene.pad0 is the tail word of the header. Word 0 is (width, height, slices, 1).
// The columns follow, one vec4 each (rgb in-scattering, transmittance).
// A zero pad0 leaves the pixel alone. History is not read.

vec3 froxel_compose(vec3 linear, uvec2 p, uvec2 dims) {
    uint at = scene.pad0;
    if (at == 0u) {
        return linear;
    }
    uvec4 head = scene.tail[at];
    uint width = head.x;
    uint height = head.y;
    if (width < 1u || height < 1u || head.w == 0u) {
        return linear;
    }
    float u = (float(p.x) + 0.5) / float(max(dims.x, 1u));
    float v = (float(p.y) + 0.5) / float(max(dims.y, 1u));
    uint x = min(uint(u * float(width)), width - 1u);
    uint y = min(uint(v * float(height)), height - 1u);
    vec4 fog = uintBitsToFloat(scene.tail[at + 1u + y * width + x]);
    return linear * fog.a + fog.rgb;
}
