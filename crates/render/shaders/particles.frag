#version 450
// Unlit card. Fade is the centre depth sample, not a per-pixel soft-particle
// test (Lorach 2007). Premultiplied. The keep pass blends this over the
// display picture. The HDR path draws the same shader into RGBA16F.
layout(location = 1) in vec3 v_albedo;
layout(location = 3) in float v_shade;
layout(location = 0) out vec4 out_color;

void main() {
    float fade = clamp(v_shade, 0.0, 1.0);
    out_color = vec4(v_albedo * fade, fade);
}
