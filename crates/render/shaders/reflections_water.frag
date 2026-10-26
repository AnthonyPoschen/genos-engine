#version 450
#extension GL_GOOGLE_include_directive : require
#include "reflections.glsl"

layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

void main() {
    vec3 normal = dot(v_normal, v_normal) > 1.0e-8 ? normalize(v_normal) : vec3(0.0, 1.0, 0.0);
    float cos_i = clamp(dot(normal, vec3(0.0, 1.0, 0.0)), 0.0, 1.0);
    float fresnel = reflection_fresnel(1.333, cos_i);
    vec3 reflected = vec3(0.72, 0.68, 0.55);
    vec3 shaded = reflection_water_shade(fresnel, reflected, v_albedo, 0.35, 1.0);
    out_color = vec4(shaded, 1.0);
}
