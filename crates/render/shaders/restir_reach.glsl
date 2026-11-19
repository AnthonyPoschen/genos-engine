#ifndef RESTIR_REACH_GLSL
#define RESTIR_REACH_GLSL

// Where a changed lamp's previous or current ball reaches. Header word 39 is
// the any-bit early-out. A reorder sets word 15 and writes no bits, so this
// returns before the previous records are read. Word 38 is the bitset. Word
// 37 is the previous record base: xyz, range, rgb, then one pad, the same
// order as the current lamp record. History stays. Its weight is
// p-hat now over p-hat then, and its count scales with that change. A zero
// sum on one side is a discrete jump. A visibility flip is not decided here.
const uint RESTIR_PREV_STRIDE = 8u;
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
            uint pat = prev_base + lamp * RESTIR_PREV_STRIDE;
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

// Current and previous records share the layout. pos.w stays 0: a point lamp.
Lamp restir_lamp_record(uint at) {
    Lamp lamp;
    lamp.pos = vec4(
        uintBitsToFloat(restir_words[at]),
        uintBitsToFloat(restir_words[at + 1u]),
        uintBitsToFloat(restir_words[at + 2u]),
        0.0);
    lamp.color = vec4(
        uintBitsToFloat(restir_words[at + 4u]),
        uintBitsToFloat(restir_words[at + 5u]),
        uintBitsToFloat(restir_words[at + 6u]),
        uintBitsToFloat(restir_words[at + 3u]));
    return lamp;
}

float restir_lamp_luma(Lamp lamp, vec3 origin, vec3 n, bool two_sided) {
    vec3 target;
    vec3 light = lamp_light(lamp, origin, n, two_sided, target);
    return dot(light, vec3(0.2126, 0.7152, 0.0722));
}

// Unshadowed sum now over the sum with changed lamps at their previous
// record. `confidence` is min/max of those sums: 1 when the light held,
// 0 when one side is empty (the light appeared or vanished). Unchanged
// lamps use the same value on both sides, so the ratio is the change and
// not which sample a reservoir happens to hold.
void restir_change_weight(vec3 p, vec3 n, bool two_sided, out float ratio, out float confidence) {
    ratio = 1.0;
    confidence = 1.0;
    if (restir_words[39u] == 0u) {
        return;
    }
    uint nlamps = restir_words[0u];
    uint base = restir_words[1u];
    uint prev_base = restir_words[37u];
    uint bits_at = restir_words[38u];
    float sum_now = 0.0;
    float sum_was = 0.0;
    [[dont_unroll]] for (uint lamp = 0u; lamp < nlamps; lamp++) {
        float now = restir_lamp_luma(restir_lamp_record(base + lamp * 8u), p, n, two_sided);
        float was = now;
        uint word = restir_words[bits_at + (lamp >> 5u)];
        if (((word >> (lamp & 31u)) & 1u) != 0u) {
            was = restir_lamp_luma(restir_lamp_record(prev_base + lamp * RESTIR_PREV_STRIDE), p, n, two_sided);
        }
        sum_now += now;
        sum_was += was;
    }
    bool now_on = sum_now > 0.0;
    bool was_on = sum_was > 0.0;
    if (now_on == was_on && !now_on) {
        return;
    }
    if (!now_on || !was_on) {
        ratio = 0.0;
        confidence = 0.0;
        return;
    }
    ratio = sum_now / sum_was;
    confidence = min(sum_now, sum_was) / max(sum_now, sum_was);
}

// A pixel this many point lamps reach is summed exactly: one shadow ray each,
// the same way the sun is traced. More than this stays on the reservoir.
// The count stops at one past the limit, so a many-lamp cell does not walk
// the rest of its list.
const uint RESTIR_EXACT_LIT = 4u;

uint restir_point_reach(vec3 origin, vec3 n, bool two_sided) {
    LampList list = lamps_at(origin);
    uint hit = 0u;
    uint begin = list.suns.y;
    uint end = begin + list.near.y;
    [[dont_unroll]] for (uint k = begin; k < end; k++) {
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, n, two_sided, target);
        if (light.r + light.g + light.b > 0.0) {
            hit += 1u;
            if (hit > RESTIR_EXACT_LIT) {
                return hit;
            }
        }
    }
    return hit;
}

bool restir_few_lamps(vec3 origin, vec3 n, bool two_sided) {
    uint hit = restir_point_reach(origin, n, two_sided);
    return hit >= 1u && hit <= RESTIR_EXACT_LIT;
}

#endif
