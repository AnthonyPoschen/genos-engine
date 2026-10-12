#version 450
layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

#extension GL_GOOGLE_include_directive : require
layout(std430, set = 0, binding = 2) readonly buffer ParticleImage {
    uint width;
    uint height;
    uint pad0;
    uint pad1;
    uint texels[];
} particle_image;

vec4 particle_texel_at(int x, int y) {
    int w = int(particle_image.width);
    int h = int(particle_image.height);
    x = clamp(x, 0, w - 1);
    y = clamp(y, 0, h - 1);
    uint packed = particle_image.texels[uint(y) * particle_image.width + uint(x)];
    float r = float(packed & 255u) / 255.0;
    float g = float((packed >> 8u) & 255u) / 255.0;
    float b = float((packed >> 16u) & 255u) / 255.0;
    float a = float((packed >> 24u) & 255u) / 255.0;
    return vec4(r, g, b, a);
}

vec4 particle_bilinear(vec2 uv) {
    float x = uv.x * float(particle_image.width) - 0.5;
    float y = uv.y * float(particle_image.height) - 0.5;
    int x0 = int(floor(x));
    int y0 = int(floor(y));
    float tx = x - float(x0);
    float ty = y - float(y0);
    vec4 c00 = particle_texel_at(x0, y0);
    vec4 c10 = particle_texel_at(x0 + 1, y0);
    vec4 c01 = particle_texel_at(x0, y0 + 1);
    vec4 c11 = particle_texel_at(x0 + 1, y0 + 1);
    return mix(mix(c00, c10, tx), mix(c01, c11, tx), ty);
}

vec4 particle_texel(vec2 uv) {
    if (uv.x < 0.0 || particle_image.width == 0u || particle_image.height == 0u) {
        return vec4(1.0);
    }
    // Spread the ember core so one screen pixel is not a bright ridge.
    vec2 step = 6.0 / vec2(float(particle_image.width), float(particle_image.height));
    vec4 sum = vec4(0.0);
    for (int dy = -1; dy <= 1; dy++) {
        for (int dx = -1; dx <= 1; dx++) {
            sum += particle_bilinear(uv + vec2(float(dx), float(dy)) * step);
        }
    }
    return sum / 9.0;
}

// Screen rectangles, lines and textured cards (the HUD and the overlay): their
// colour as given, through the particle image when they carry one. The scene's
// surfaces are lit by the lighting passes (lighting.md); they never reach here.
void main() {
    vec4 texel = particle_texel(v_uv);
    if (v_uv.x >= 0.0 && texel.a < 0.04) {
        discard;
    }
    if (v_shade >= 0.5) {
        discard;
    }
    vec3 albedo = v_albedo * texel.rgb;
    out_color = vec4(albedo * texel.a, texel.a);
}
