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

// n_cross_h_sq is |n x h|^2 = 1 - (n.h)^2, passed in from the vectors.
// The textbook den = (n.h)^2 (a2 - 1) + 1 cancels near n.h = 1: with
// alpha = 0.01, a2 = 1e-4, and one ulp of n.h (from normalize/rsqrt)
// shifts den by ~1.2e-7, a 0.27% D error. |n x h|^2 has no cancellation.
float pbr_d_ggx(float n_dot_h, float n_cross_h_sq, float alpha) {
    if (n_dot_h <= 0.0) {
        return 0.0;
    }
    float a2 = alpha * alpha;
    float den = n_dot_h * n_dot_h * a2 + n_cross_h_sq;
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

// Specular lobe F·D·V only, without the n·l factor. Dielectric F0 is 0.04.
// The direct pass adds this to the engine Lambert weight. It does not replace
// that weight with base/π: the tuned picture uses 0.254647909, and base/π
// lifts every dielectric into the tone shoulder.
vec3 pbr_specular(vec3 base_color, float metallic, float roughness,
                  float n_dot_l, float n_dot_v, float n_dot_h, float n_cross_h_sq,
                  float v_dot_h) {
    if (n_dot_l <= 0.0 || n_dot_v <= 0.0) {
        return vec3(0.0);
    }
    float alpha = pbr_alpha(roughness);
    float dv = pbr_d_ggx(n_dot_h, n_cross_h_sq, alpha) * pbr_v_smith_ggx_correlated(n_dot_l, n_dot_v, alpha);
    float m = clamp(metallic, 0.0, 1.0);
    vec3 f = mix(
        vec3(pbr_fresnel_schlick(PBR_DIELECTRIC_F0, v_dot_h)),
        vec3(
            pbr_fresnel_schlick(base_color.r, v_dot_h),
            pbr_fresnel_schlick(base_color.g, v_dot_h),
            pbr_fresnel_schlick(base_color.b, v_dot_h)),
        m);
    return f * dv;
}

// f(l, v) per channel, without the n·l factor.
vec3 pbr_eval(vec3 base_color, float metallic, float roughness,
              float n_dot_l, float n_dot_v, float n_dot_h, float n_cross_h_sq,
              float v_dot_h) {
    if (n_dot_l <= 0.0 || n_dot_v <= 0.0) {
        return vec3(0.0);
    }
    float alpha = pbr_alpha(roughness);
    float dv = pbr_d_ggx(n_dot_h, n_cross_h_sq, alpha) * pbr_v_smith_ggx_correlated(n_dot_l, n_dot_v, alpha);
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
