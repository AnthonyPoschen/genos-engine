#version 450
// Six vertices, one instance per compacted particle. The card faces the camera.
// right.w is the card width in metres.
layout(location = 0) out vec3 v_pos;
layout(location = 1) out vec3 v_albedo;
layout(location = 3) out float v_shade;

layout(push_constant) uniform Push {
    mat4 view_proj;
    vec4 right;
    vec4 up;
} pc;

struct Compact {
    vec4 position_age;
    vec4 velocity_life;
    vec4 albedo_fade;
};

layout(std430, set = 0, binding = 0) readonly buffer CompactBuf {
    Compact items[];
} compact;

const float HALF = 0.5;

void main() {
    int id = gl_VertexIndex;
    float x = (id == 1 || id == 4 || id == 5) ? 1.0 : -1.0;
    float y = (id == 2 || id == 3 || id == 5) ? 1.0 : -1.0;
    Compact item = compact.items[gl_InstanceIndex];
    float size = pc.right.w;
    vec3 world = item.position_age.xyz
        + pc.right.xyz * (x * size * HALF)
        + pc.up.xyz * (y * size * HALF);
    gl_Position = pc.view_proj * vec4(world, 1.0);
    v_pos = world;
    v_albedo = item.albedo_fade.rgb;
    v_shade = item.albedo_fade.a;
}
