#version 450
layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

layout(set = 0, binding = 0) uniform sampler2D refl_color;
layout(set = 0, binding = 1) uniform sampler2D refl_depth;

layout(push_constant) uniform Pc {
    layout(offset = 64) vec4 params;
    // xy is the crop. A guard band draws this pass into the lit picture.
    layout(offset = 80) vec4 frame;
} pc;

// The depth test uses the reflector's own window depth. The shader then writes
// the reflected hit's window depth, which is farther than the surface.
layout(early_fragment_tests) in;

const float SAMPLE_ON = 0.5;
const float CLEAR_DEPTH = 0.9995;

void main() {
    if (pc.params.z < SAMPLE_ON) {
        out_color = vec4(v_albedo, 1.0);
        gl_FragDepth = gl_FragCoord.z;
        return;
    }
    vec2 uv = (gl_FragCoord.xy - pc.frame.xy) / max(pc.params.xy, vec2(1.0));
    float depth = texture(refl_depth, uv).r;
    if (depth > CLEAR_DEPTH) {
        out_color = vec4(v_albedo, 1.0);
        gl_FragDepth = gl_FragCoord.z;
        return;
    }
    out_color = vec4(texture(refl_color, uv).rgb, 1.0);
    gl_FragDepth = depth;
}
