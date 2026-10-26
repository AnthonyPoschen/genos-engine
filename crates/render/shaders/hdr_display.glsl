// AgX lattice. Filled by agx_rec709. N = 32, log2 from -12 to 10, x-fastest.
// display mode 1 samples it. Mode 0 is the legacy shoulder. Mode 2 is ACES,
// which is not the Academy CTL, so the caller keeps the shoulder.
layout(std430, set = 0, binding = 15) readonly buffer AgxLut {
    vec4 texel[];
} agx_lut;

vec3 agx_fetch(int x, int y, int z) {
    int index = (z * 32 + y) * 32 + x;
    return agx_lut.texel[index].rgb;
}

vec3 agx_sample(vec3 c) {
    vec3 p = (log2(max(c, vec3(1e-8))) - vec3(-12.0)) / 22.0;
    p = clamp(p, 0.0, 1.0);
    vec3 g = p * 31.0;
    ivec3 i0 = clamp(ivec3(floor(g)), ivec3(0), ivec3(31));
    ivec3 i1 = min(i0 + ivec3(1), ivec3(31));
    vec3 f = g - vec3(i0);
    vec3 c000 = agx_fetch(i0.x, i0.y, i0.z);
    vec3 c100 = agx_fetch(i1.x, i0.y, i0.z);
    vec3 c010 = agx_fetch(i0.x, i1.y, i0.z);
    vec3 c110 = agx_fetch(i1.x, i1.y, i0.z);
    vec3 c001 = agx_fetch(i0.x, i0.y, i1.z);
    vec3 c101 = agx_fetch(i1.x, i0.y, i1.z);
    vec3 c011 = agx_fetch(i0.x, i1.y, i1.z);
    vec3 c111 = agx_fetch(i1.x, i1.y, i1.z);
    vec3 c00 = mix(c000, c100, f.x);
    vec3 c10 = mix(c010, c110, f.x);
    vec3 c01 = mix(c001, c101, f.x);
    vec3 c11 = mix(c011, c111, f.x);
    vec3 c0 = mix(c00, c10, f.y);
    vec3 c1 = mix(c01, c11, f.y);
    return mix(c0, c1, f.z);
}
