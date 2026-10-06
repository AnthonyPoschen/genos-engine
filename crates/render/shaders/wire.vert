#version 450
layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_albedo;
layout(location = 2) in vec3 in_normal;
layout(location = 3) in float in_shade;
layout(location = 4) in vec2 in_uv;
layout(push_constant) uniform Push {
    mat4 view_proj;
} pc;
void main() {
    gl_Position = pc.view_proj * vec4(in_pos, 1.0);
}
