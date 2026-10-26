// Mirror of crates/math/src/optics.rs. Reflection passes include this file.
// Directions into reflection_refract are unit length. Portal bases are
// orthonormal, so the inverse basis is the transpose. MIRROR_FRESNEL_CUTOFF
// matches the Rust constant.

const float MIRROR_FRESNEL_CUTOFF = 0.02;

vec3 reflection_mirror_dir(vec3 incident, vec3 normal) {
    return incident - 2.0 * dot(normal, incident) * normal;
}

vec3 reflection_mirror_point(vec3 point, vec3 normal) {
    return point - 2.0 * dot(point, normal) * normal;
}

float reflection_fresnel(float eta_t_over_eta_i, float cos_i) {
    cos_i = clamp(cos_i, 0.0, 1.0);
    float sin2_i = max(0.0, 1.0 - cos_i * cos_i);
    float sin2_t = sin2_i / (eta_t_over_eta_i * eta_t_over_eta_i);
    if (sin2_t >= 1.0) {
        return 1.0;
    }
    float cos_t = sqrt(1.0 - sin2_t);
    float eta = eta_t_over_eta_i;
    float rs = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    float rp = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    return 0.5 * (rs * rs + rp * rp);
}

bool reflection_refract(vec3 incident, vec3 normal, float eta, out vec3 transmitted) {
    float along = dot(normal, incident);
    float k = 1.0 - eta * eta * (1.0 - along * along);
    if (k < 0.0) {
        transmitted = vec3(0.0);
        return false;
    }
    float scale = eta * along + sqrt(k);
    transmitted = eta * incident - scale * normal;
    return true;
}

float reflection_ggx_ndf(float n_dot_h, float alpha) {
    float mu = clamp(n_dot_h, 0.0, 1.0);
    float a2 = alpha * alpha;
    float den = mu * mu * (a2 - 1.0) + 1.0;
    return a2 / (3.141592653589793 * den * den);
}

float reflection_beer(float sigma, float distance) {
    return exp(-sigma * distance);
}

float reflection_flat_water(float ior) {
    return 1.0 - reflection_fresnel(ior, 1.0);
}

vec3 reflection_portal_point(
    vec3 point,
    vec3 from_origin,
    mat3 from_basis,
    vec3 to_origin,
    mat3 to_basis
) {
    vec3 local = transpose(from_basis) * (point - from_origin);
    vec3 flipped = vec3(local.x, local.y, -local.z);
    return to_origin + to_basis * flipped;
}

uint reflection_mirror_views(const float fresnel[8], uint count, uint cap, float cutoff) {
    float weight = 1.0;
    uint emitted = 0u;
    uint n = min(count, 8u);
    for (uint i = 0u; i < n; i++) {
        if (emitted >= cap) {
            break;
        }
        weight *= fresnel[i];
        if (weight < cutoff) {
            break;
        }
        emitted += 1u;
    }
    return emitted;
}
