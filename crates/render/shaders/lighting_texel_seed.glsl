// ADR 0039 stage 3. One cell's colour SH-L1, evaluated at the texel normal,
// is a low-confidence seed. The seed never counts as the texel's own samples,
// and this file has no cell store to write. `LIGHTING_SH_A1` and
// `LIGHTING_SH_Y1` come from the including shader (`lighting_cache.glsl`).

const float LIGHTING_TEXEL_SEED_MIN = 0.05;
const float LIGHTING_TEXEL_SEED_MAX = 1.0;
const float LIGHTING_TEXEL_OWN_MARK = 0.5;

vec3 lighting_texel_seed_tilt(vec3 e_axis, vec3 axis, vec3 n, vec3 cx, vec3 cy, vec3 cz) {
    vec3 d = n - axis;
    return max(e_axis + (LIGHTING_SH_A1 * LIGHTING_SH_Y1) * (cx * d.x + cy * d.y + cz * d.z), vec3(0.0));
}

struct LightingTexelSeed {
    vec3 own;
    float own_samples;
    vec3 seed;
    float confidence;
};

// `as_samples` is the must-fail control: the seed becomes the own mean and
// survives `lighting_texel_seed_clear`. Production never sets it.
LightingTexelSeed lighting_texel_seed_apply(LightingTexelSeed s, vec3 value, float confidence, bool as_samples, float seed_sets) {
    if (as_samples) {
        s.own = value;
        s.own_samples = seed_sets;
        s.seed = vec3(0.0);
        s.confidence = 0.0;
    } else {
        s.seed = max(value, vec3(0.0));
        s.confidence = clamp(confidence, LIGHTING_TEXEL_SEED_MIN, LIGHTING_TEXEL_SEED_MAX);
    }
    return s;
}

LightingTexelSeed lighting_texel_seed_clear(LightingTexelSeed s) {
    s.seed = vec3(0.0);
    s.confidence = 0.0;
    return s;
}

// Default fine texel (SVGF on, no fresh field): one own sample shows that
// sample, and none shows the seed. The sparse mix in `lighting_sc_shown` is
// a different arm.
vec3 lighting_texel_seed_shown(LightingTexelSeed s) {
    if (s.own_samples > LIGHTING_TEXEL_OWN_MARK) {
        return s.own;
    }
    if (s.confidence > 0.0) {
        return s.seed;
    }
    return vec3(0.0);
}
