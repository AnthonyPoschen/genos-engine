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
// The depth pass (gpu.rs make_depth_pipeline) and the shading pass draw the same
// shapes; both must land on the same depth.
invariant gl_Position;
layout(push_constant) uniform Push {
    mat4 view_proj;
} pc;
struct Instance {
    mat4 model;
    vec4 color;
};
layout(std430, set = 0, binding = 3) readonly buffer Instances {
    Instance items[];
} instances;
void main() {
    Instance item = instances.items[gl_InstanceIndex];
    vec4 world = item.model * vec4(in_pos, 1.0);
    vec3 spun = mat3(item.model) * in_normal;
    float len = length(spun);
    gl_Position = pc.view_proj * world;
    v_pos = world.xyz;
    v_albedo = in_albedo * item.color.rgb;
    v_normal = len > 1e-6 ? spun / len : spun;
    v_shade = in_shade;
    v_uv = in_uv;
}
