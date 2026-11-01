// Material atlases (gpu.rs). Binding 4 is the device-local base colour image
// (sRGB bytes stored as unorm, linear alpha). Binding 7 is the device-local
// tangent-space normal. The transfer queue copies a resident mip and blits it
// into the image. A rect with z <= 0, or a negative uv, keeps the vertex colour
// and the vertex normal.
layout(set = 0, binding = 4, rgba8) uniform readonly image2D material_color;
layout(set = 0, binding = 7, rgba8) uniform readonly image2D material_normal;

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
    ivec2 size = imageSize(material_color);
    if (uv.x < 0.0 || rect.z <= 0.5 || size.x <= 0 || size.y <= 0) {
        return vec4(1.0);
    }
    return imageLoad(material_color, material_coord(size, rect, uv));
}

float material_normal_alpha(vec4 rect, vec2 uv) {
    ivec2 size = imageSize(material_normal);
    const float material_rect_present = 0.5;
    if (uv.x < 0.0 || rect.z <= material_rect_present || size.x <= 0 || size.y <= 0) {
        return 1.0;
    }
    return imageLoad(material_normal, material_coord(size, rect, uv)).a;
}

vec3 material_normal_at(vec4 rect, vec2 uv) {
    ivec2 size = imageSize(material_normal);
    if (uv.x < 0.0 || rect.z <= 0.5 || size.x <= 0 || size.y <= 0) {
        return vec3(0.5, 0.5, 1.0);
    }
    return imageLoad(material_normal, material_coord(size, rect, uv)).rgb;
}

float material_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}
