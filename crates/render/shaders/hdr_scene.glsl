// Scene-referred radiance, RGBA16F. w is the eye distance in metres.
layout(std430, set = 0, binding = 13) restrict buffer SceneReferred {
    uint words[];
} scene_lin;

void scene_store(uint index, vec4 v) {
    scene_lin.words[index * 2u] = packHalf2x16(v.rg);
    scene_lin.words[index * 2u + 1u] = packHalf2x16(vec2(v.b, v.a));
}

vec4 scene_load(uint index) {
    vec2 rg = unpackHalf2x16(scene_lin.words[index * 2u]);
    vec2 ba = unpackHalf2x16(scene_lin.words[index * 2u + 1u]);
    return vec4(rg, ba);
}
