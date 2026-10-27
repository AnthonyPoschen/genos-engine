// Bloom add from bloom_pyramid, before exposure. Intensity 0 does not read it.
// AgX's lattice lives in hdr_agx.comp. Declaring it here pulled the buffer into
// the GI compose shader, and lavapipe then moved the bounce between frames.
layout(std430, set = 0, binding = 16) restrict readonly buffer BloomAdd {
    vec4 px[];
} bloom_add;

vec3 add_bloom(vec3 c, uint index) {
    if (hdr.v.z > 0.0) {
        c += bloom_add.px[index].rgb * hdr.v.z;
    }
    return c;
}
