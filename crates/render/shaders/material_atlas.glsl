// Material atlases (gpu.rs). Binding 4 is the device-local base colour image.
// Binding 7 is the tangent-space normal. The source path stores sRGB bytes in
// an RGBA8 unorm image. The cooked path (ADR 0035) stores BC7 sRGB and BC5.
// Binding 1 says which: color_linear is 1 when a texel fetch is already linear
// (BC7 sRGB). color_lod and normal_lod are the resident mip. A rect with
// z <= 0, or a negative uv, keeps the vertex colour and the vertex normal.
layout(std140, set = 0, binding = 1) uniform AtlasMode {
    uint color_linear;
    uint color_lod;
    uint normal_lod;
    uint pad_atlas;
} atlas_mode;
layout(set = 0, binding = 4) uniform sampler2D material_color;
layout(set = 0, binding = 7) uniform sampler2D material_normal;

ivec2 material_coord(ivec2 size, vec4 rect, vec2 uv) {
    float u = uv.x - floor(uv.x);
    float v = uv.y - floor(uv.y);
    int x = int(rect.x) + int(min(u * rect.z, rect.z - 1.0));
    int y = int(rect.y) + int(min(v * rect.w, rect.w - 1.0));
    x = clamp(x, 0, size.x - 1);
    y = clamp(y, 0, size.y - 1);
    return ivec2(x, y);
}

vec4 material_fetch(vec4 rect, vec2 uv) {
    ivec2 size = textureSize(material_color, 0);
    if (uv.x < 0.0 || rect.z <= 0.5 || size.x <= 0 || size.y <= 0) {
        return vec4(1.0);
    }
    int lod = int(atlas_mode.color_lod);
    ivec2 c = material_coord(size, rect, uv) >> lod;
    ivec2 lod_size = textureSize(material_color, lod);
    c = clamp(c, ivec2(0), lod_size - ivec2(1));
    return texelFetch(material_color, c, lod);
}

vec3 material_normal_at(vec4 rect, vec2 uv) {
    ivec2 size = textureSize(material_normal, 0);
    if (uv.x < 0.0 || rect.z <= 0.5 || size.x <= 0 || size.y <= 0) {
        return vec3(0.5, 0.5, 1.0);
    }
    int lod = int(atlas_mode.normal_lod);
    ivec2 c = material_coord(size, rect, uv) >> lod;
    ivec2 lod_size = textureSize(material_normal, lod);
    c = clamp(c, ivec2(0), lod_size - ivec2(1));
    return texelFetch(material_normal, c, lod).rgb;
}

float material_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}
