// glTF 2.0 metallic-roughness BRDF (Appendix B). Same formulas as
// genos_math::brdf; pbr_check.comp compares the two on the GPU.
// Forward and G-buffer shades include this file when they take the PBR lobe.
#ifndef PBR_BRDF_GLSL
#define PBR_BRDF_GLSL

const float PBR_PI = 3.14159265358979;
const float PBR_DIELECTRIC_F0 = 0.04;
// V = G2 / (4 n·l n·v) folds to 0.5 / (gv + gl).
const float PBR_V_NUMERATOR = 0.5;

float pbr_alpha(float roughness) {
    float r = clamp(roughness, 0.0, 1.0);
    return r * r;
}

float pbr_d_ggx(float n_dot_h, float alpha) {
    if (n_dot_h <= 0.0) {
        return 0.0;
    }
    float a2 = alpha * alpha;
    float den = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PBR_PI * den * den);
}

float pbr_v_smith_ggx_correlated(float n_dot_l, float n_dot_v, float alpha) {
    if (n_dot_l <= 0.0 || n_dot_v <= 0.0) {
        return 0.0;
    }
    float a2 = alpha * alpha;
    float gv = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
    float gl = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
    float s = gv + gl;
    return s > 0.0 ? PBR_V_NUMERATOR / s : 0.0;
}

float pbr_fresnel_schlick(float f0, float v_dot_h) {
    float m = 1.0 - clamp(v_dot_h, 0.0, 1.0);
    float m2 = m * m;
    return f0 + (1.0 - f0) * (m2 * m2 * m);
}

// f(l, v) per channel, without the n·l factor.
vec3 pbr_eval(vec3 base_color, float metallic, float roughness,
              float n_dot_l, float n_dot_v, float n_dot_h, float v_dot_h) {
    if (n_dot_l <= 0.0 || n_dot_v <= 0.0) {
        return vec3(0.0);
    }
    float alpha = pbr_alpha(roughness);
    float dv = pbr_d_ggx(n_dot_h, alpha) * pbr_v_smith_ggx_correlated(n_dot_l, n_dot_v, alpha);
    float m = clamp(metallic, 0.0, 1.0);
    float fd = pbr_fresnel_schlick(PBR_DIELECTRIC_F0, v_dot_h);
    vec3 dielectric = (1.0 - fd) * (base_color / PBR_PI) + fd * dv;
    vec3 metal = vec3(
        pbr_fresnel_schlick(base_color.r, v_dot_h),
        pbr_fresnel_schlick(base_color.g, v_dot_h),
        pbr_fresnel_schlick(base_color.b, v_dot_h)) * dv;
    return mix(dielectric, metal, m);
}

#endif
