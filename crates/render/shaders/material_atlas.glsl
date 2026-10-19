// Material atlases (gpu.rs). Binding 4 is base colour (sRGB rgb, linear alpha).
// Binding 7 is a tangent-space normal (linear). A rect with z <= 0, or a
// negative uv, keeps the vertex colour and the vertex normal.
layout(std430, set = 0, binding = 4) readonly buffer MaterialColor {
    uint width;
    uint height;
    uint pad0;
    uint pad1;
    uint texels[];
} material_color;

vec4 material_fetch(vec4 rect, vec2 uv) {
    if (uv.x < 0.0 || rect.z <= 0.5 || material_color.width == 0u || material_color.height == 0u) {
        return vec4(1.0);
    }
    float u = uv.x - floor(uv.x);
    float v = uv.y - floor(uv.y);
    uint x = uint(rect.x) + uint(min(u * rect.z, rect.z - 1.0));
    uint y = uint(rect.y) + uint(min(v * rect.w, rect.w - 1.0));
    x = min(x, material_color.width - 1u);
    y = min(y, material_color.height - 1u);
    uint packed = material_color.texels[y * material_color.width + x];
    return vec4(
        float(packed & 255u) / 255.0,
        float((packed >> 8u) & 255u) / 255.0,
        float((packed >> 16u) & 255u) / 255.0,
        float((packed >> 24u) & 255u) / 255.0);
}

float material_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}
