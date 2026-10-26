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

float reflection_sgn(float value) {
    return value < 0.0 ? -1.0 : 1.0;
}

uint reflection_ray_cap(uint preset) {
    if (preset <= 1u) {
        return 0u;
    }
    if (preset == 2u) {
        return 32000u;
    }
    return 128000u;
}

uint reflection_view_cap(uint preset) {
    if (preset == 0u) {
        return 0u;
    }
    if (preset == 1u) {
        return 1u;
    }
    if (preset == 2u) {
        return 2u;
    }
    return 3u;
}

bool reflection_plane_front(vec3 point, vec3 origin, vec3 normal) {
    return dot(normal, point - origin) >= 0.0;
}

mat4 reflection_inverse(mat4 m) {
    float aug[32];
    for (int row = 0; row < 4; row++) {
        for (int col = 0; col < 4; col++) {
            aug[row * 8 + col] = m[col][row];
            aug[row * 8 + 4 + col] = col == row ? 1.0 : 0.0;
        }
    }
    for (int col = 0; col < 4; col++) {
        int pivot = col;
        float best = abs(aug[col * 8 + col]);
        for (int row = col + 1; row < 4; row++) {
            float value = abs(aug[row * 8 + col]);
            if (value > best) {
                best = value;
                pivot = row;
            }
        }
        if (best < 1.0e-12) {
            return mat4(1.0);
        }
        if (pivot != col) {
            for (int k = 0; k < 8; k++) {
                float swap = aug[col * 8 + k];
                aug[col * 8 + k] = aug[pivot * 8 + k];
                aug[pivot * 8 + k] = swap;
            }
        }
        float div = aug[col * 8 + col];
        for (int k = 0; k < 8; k++) {
            aug[col * 8 + k] /= div;
        }
        for (int row = 0; row < 4; row++) {
            if (row == col) {
                continue;
            }
            float factor = aug[row * 8 + col];
            for (int k = 0; k < 8; k++) {
                aug[row * 8 + k] -= factor * aug[col * 8 + k];
            }
        }
    }
    mat4 inv = mat4(1.0);
    for (int row = 0; row < 4; row++) {
        for (int col = 0; col < 4; col++) {
            inv[col][row] = aug[row * 8 + 4 + col];
        }
    }
    return inv;
}

mat4 reflection_parity_projection() {
    float h = 1.0 / tan(radians(60.0) * 0.5);
    float near_z = 0.05;
    float far_z = 200.0;
    mat4 proj = mat4(0.0);
    proj[0][0] = h;
    proj[1][1] = -h;
    proj[2][2] = far_z / (near_z - far_z);
    proj[2][3] = -1.0;
    proj[3][2] = (far_z * near_z) / (near_z - far_z);
    return proj;
}

vec4 reflection_oblique_row(mat4 projection, vec4 plane) {
    vec4 corner = vec4(reflection_sgn(plane.x), reflection_sgn(plane.y), 1.0, 1.0);
    vec4 q = reflection_inverse(projection) * corner;
    float denom = dot(plane, q);
    float scale = abs(denom) < 1.0e-12 ? 0.0 : 1.0 / denom;
    return plane * scale;
}

void reflection_gerstner(
    vec2 direction,
    float amplitude,
    float wavelength,
    float speed,
    float steepness,
    vec2 xz,
    float time,
    out vec3 displaced,
    out vec3 normal
) {
    float len = length(direction);
    vec2 axis = len <= 1.0e-8 ? vec2(1.0, 0.0) : direction / len;
    float wave_number = 6.283185307179586 / max(wavelength, 1.0e-4);
    float phase = wave_number * (dot(axis, xz) - speed * time);
    float sine = sin(phase);
    float cosine = cos(phase);
    displaced = vec3(xz.x, 0.0, xz.y);
    displaced.x += steepness * amplitude * axis.x * cosine;
    displaced.y += amplitude * sine;
    displaced.z += steepness * amplitude * axis.y * cosine;
    normal = vec3(0.0, 1.0, 0.0);
    float slope = wave_number * amplitude;
    normal.x -= axis.x * slope * cosine;
    normal.y -= steepness * slope * sine;
    normal.z -= axis.y * slope * cosine;
    normal = normalize(normal);
}

float reflection_shoreline(float deep, float depth, float cap) {
    float limit = max(cap * depth, 0.0);
    return clamp(deep, -limit, limit);
}

vec3 reflection_water_shade(float fresnel, vec3 reflected, vec3 transmitted, float sigma, float distance) {
    float weight = clamp(fresnel, 0.0, 1.0);
    float absorb = reflection_beer(sigma, distance);
    return reflected * weight + transmitted * ((1.0 - weight) * absorb);
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
