#version 450
layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_albedo;
layout(location = 2) in vec3 in_normal;
layout(location = 3) in float in_shade;
layout(location = 4) in vec2 in_uv;
layout(location = 0) out vec3 v_pos;
layout(location = 1) out vec3 v_albedo;
layout(location = 2) out vec3 v_normal;
layout(location = 3) out float v_shade;
layout(location = 4) out vec2 v_uv;
layout(push_constant) uniform Push {
    mat4 view_proj;
} pc;
void main() {
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
    v_pos = in_pos;
    v_albedo = in_albedo;
    v_normal = in_normal;
    v_shade = in_shade;
    v_uv = in_uv;
}
