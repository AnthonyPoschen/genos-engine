// Scene-referred radiance, RGBA16F. w is the eye distance in metres.
layout(std430, set = 0, binding = 13) restrict buffer SceneReferred {
    uint words[];
} scene_lin;

// Largest finite binary16. packHalf2x16 turns anything larger into +inf.
const float HALF_MAX = 65504.0;

vec2 finite_half2(vec2 v) {
    return clamp(v, vec2(-HALF_MAX), vec2(HALF_MAX));
}

void scene_store(uint index, vec4 v) {
    scene_lin.words[index * 2u] = packHalf2x16(finite_half2(v.rg));
    scene_lin.words[index * 2u + 1u] = packHalf2x16(finite_half2(v.ba));
}

vec4 scene_load(uint index) {
    vec2 rg = unpackHalf2x16(scene_lin.words[index * 2u]);
    vec2 ba = unpackHalf2x16(scene_lin.words[index * 2u + 1u]);
    return vec4(rg, ba);
}
