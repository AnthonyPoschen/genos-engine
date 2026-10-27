#version 450
#extension GL_GOOGLE_include_directive : require
#include "reflections.glsl"

layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

// Projected GGX integral 2π ∫ D(μ) μ dμ. A normalized distribution lands on 1.
void main() {
    const float alpha = 0.35;
    const int steps = 64;
    const float HALF_STEP = 0.5;
    const float TWO_PI = 6.283185307179586;
    float sum = 0.0;
    for (int i = 0; i < steps; i++) {
        float mu = (float(i) + HALF_STEP) / float(steps);
        sum += reflection_ggx_ndf(mu, alpha) * mu / float(steps);
    }
    float energy = sum * TWO_PI;
    out_color = vec4(energy, energy, energy, 1.0);
}
