#ifndef RESTIR_REACH_GLSL
#define RESTIR_REACH_GLSL

// Samples of direct-light history kept where a changed lamp reaches the
// pixel and its unshadowed target barely moved. The new sample then weighs
// 1/13, so the previous average falls by about 1/e in 12 frames and is
// mostly gone by 16. One sample is the salt. A larger move shortens this
// window by the 12-bit log step count, and 8 steps (about 9%) drops the
// mean. Header word 39 is the any-bit early-out. Word 38 is the bitset.
const uint RESTIR_CHANGE_FADE = 12u;
const uint RESTIR_CHANGE_DROP_STEPS = 8u;

// True when a point lamp flagged in the changed-lamp bitset can reach p.
// The test is the lamp's range sphere (lamp_reaches), not which lamp a
// reservoir sample holds. A cut keyed to the sample darkened a wall by
// about 20%. A zero range reaches everywhere, the same rule as a flame.
// Word 39 clear skips the walk.
bool restir_changed_reaches(vec3 p) {
    if (restir_words[39u] == 0u) {
        return false;
    }
    uint n = restir_words[0u];
    uint base = restir_words[1u];
    uint bits_at = restir_words[38u];
    uint lamp = 0u;
    while (lamp < n) {
        uint word = restir_words[bits_at + (lamp >> 5u)];
        if (word == 0u) {
            lamp = (lamp & ~31u) + 32u;
            continue;
        }
        if (((word >> (lamp & 31u)) & 1u) != 0u) {
            uint at = base + lamp * 8u;
            Lamp src;
            src.pos = vec4(
                uintBitsToFloat(restir_words[at]),
                uintBitsToFloat(restir_words[at + 1u]),
                uintBitsToFloat(restir_words[at + 2u]),
                0.0);
            src.color = vec4(
                uintBitsToFloat(restir_words[at + 4u]),
                uintBitsToFloat(restir_words[at + 5u]),
                uintBitsToFloat(restir_words[at + 6u]),
                uintBitsToFloat(restir_words[at + 3u]));
            if (lamp_reaches(src, p)) {
                return true;
            }
        }
        lamp += 1u;
    }
    return false;
}

#endif
