#ifndef RESTIR_REACH_GLSL
#define RESTIR_REACH_GLSL

// Samples kept where a changed lamp still reaches the shaded point. The new
// sample then weighs 1/13. A pixel the lamp used to reach, and no longer
// does, keeps two samples so that light leaves inside the same fade instead
// of sitting in a frozen mean. Eight 12-bit log steps (about 9%) of the
// stored sample still drop the mean. Header word 39 is the any-bit early-out.
// Word 38 is the bitset. Word 37 is the previous-ball base (xyz + range).
const uint RESTIR_CHANGE_FADE = 12u;
const uint RESTIR_CHANGE_LEAVE = 2u;
const uint RESTIR_CHANGE_DROP_STEPS = 8u;
// Bit 0: the lamp's ball this frame. Bit 1: its ball last frame.
const uint RESTIR_REACH_NOW = 1u;
const uint RESTIR_REACH_BEFORE = 2u;

// Finite ball only. A range of 0 is a lamp that is off, not the flame rule
// in lamp_reaches (range <= 0 lights everywhere). Colour-to-0 therefore
// fades the previous ball and does not touch pixels that ball missed.
bool restir_finite_ball(vec3 center, float range, vec3 p) {
    if (!(range > 0.0)) {
        return false;
    }
    vec3 d = center - p;
    return dot(d, d) <= range * range;
}

// Union of the changed lamp's previous and current balls at p. p is the
// shaded origin (surface + normal * 0.02), the point lamp_light uses.
// The test does not look at which lamp a reservoir sample holds.
uint restir_changed_reach(vec3 p) {
    if (restir_words[39u] == 0u) {
        return 0u;
    }
    uint n = restir_words[0u];
    uint base = restir_words[1u];
    uint prev_base = restir_words[37u];
    uint bits_at = restir_words[38u];
    uint hit = 0u;
    uint lamp = 0u;
    while (lamp < n) {
        uint word = restir_words[bits_at + (lamp >> 5u)];
        if (word == 0u) {
            lamp = (lamp & ~31u) + 32u;
            continue;
        }
        if (((word >> (lamp & 31u)) & 1u) != 0u) {
            uint at = base + lamp * 8u;
            vec3 now = vec3(
                uintBitsToFloat(restir_words[at]),
                uintBitsToFloat(restir_words[at + 1u]),
                uintBitsToFloat(restir_words[at + 2u]));
            float now_range = uintBitsToFloat(restir_words[at + 3u]);
            uint pat = prev_base + lamp * 4u;
            vec3 was = vec3(
                uintBitsToFloat(restir_words[pat]),
                uintBitsToFloat(restir_words[pat + 1u]),
                uintBitsToFloat(restir_words[pat + 2u]));
            float was_range = uintBitsToFloat(restir_words[pat + 3u]);
            if (restir_finite_ball(now, now_range, p)) {
                hit |= RESTIR_REACH_NOW;
            }
            if (restir_finite_ball(was, was_range, p)) {
                hit |= RESTIR_REACH_BEFORE;
            }
            if (hit == (RESTIR_REACH_NOW | RESTIR_REACH_BEFORE)) {
                return hit;
            }
        }
        lamp += 1u;
    }
    return hit;
}

bool restir_changed_reaches(vec3 p) {
    return restir_changed_reach(p) != 0u;
}

#endif
