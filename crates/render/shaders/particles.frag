#version 450
// Unlit card in the keep pass. Fade is the centre depth sample, not a
// per-pixel soft-particle test (Lorach 2007). Premultiplied so the keep-pass
// blend darkens toward the picture behind the card.
layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 3) in float v_shade;
layout(location = 0) out vec4 out_color;

void main() {
    float fade = clamp(v_shade, 0.0, 1.0);
    out_color = vec4(v_albedo * fade, fade);
}
