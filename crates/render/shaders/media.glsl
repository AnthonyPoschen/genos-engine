// Closed forms for a participating medium. Mirrors crates/math/src/media.rs.
// HEIGHT_FOG_HORIZONTAL_COS is 1e-4 there and here.

const float MEDIA_HORIZONTAL_COS = 1.0e-4;

float media_transmittance(float optical_depth) {
    return exp(-optical_depth);
}

float media_height_density(float sigma0, float thickness, float y, float base_height) {
    if (thickness <= 0.0) {
        return sigma0;
    }
    return sigma0 * exp(-(y - base_height) / thickness);
}

float media_phase_hg(float g, float mu) {
    float denom = 1.0 + g * g - 2.0 * g * mu;
    if (denom <= 0.0) {
        return 0.0;
    }
    return (1.0 - g * g) / (4.0 * 3.141592653589793 * pow(denom, 1.5));
}

float media_homogeneous_inscatter(float sigma, float source, float distance) {
    if (abs(sigma) <= 1.0e-8) {
        return source * distance;
    }
    return (source / sigma) * (1.0 - media_transmittance(sigma * distance));
}

float media_height_optical_depth(
    float sigma0,
    float thickness,
    float base_height,
    float y0,
    float dy,
    float cos_theta,
    float distance
) {
    float sigma_y0 = media_height_density(sigma0, thickness, y0, base_height);
    if (abs(cos_theta) < MEDIA_HORIZONTAL_COS || thickness <= 0.0) {
        return sigma_y0 * abs(distance);
    }
    return sigma_y0 * thickness * (1.0 - exp(-dy / thickness)) / cos_theta;
}
