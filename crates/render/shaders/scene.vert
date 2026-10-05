#version 450
layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_color;
layout(location = 0) out vec3 v_color;
layout(push_constant) uniform Push {
    mat4 view_proj;
} pc;
void main() {
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
    v_color = in_color;
}
