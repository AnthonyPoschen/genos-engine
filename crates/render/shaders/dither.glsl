// Dither for the 8-bit writes. The swapchain (and the anti-aliasing buffers) hold
// 8 bits per channel, so a smooth dim gradient (lamp falloff on a floor, a dim wall)
// shows as rings or bands one code apart. Adding zero-mean noise of up to one code
// before rounding turns those steps into fine grain with the same average.

// Uniform hash in [0, 1) from integer pixel coordinates (PCG-style).
float dither_hash(uvec2 p, uint salt) {
    uint v = p.x * 1664525u + p.y * 1013904223u + salt * 2654435769u;
    v ^= v >> 16u;
    v *= 0x7feb352du;
    v ^= v >> 15u;
    v *= 0x846ca68bu;
    v ^= v >> 16u;
    return float(v >> 8u) * (1.0 / 16777216.0);
}

// Triangular noise in (-1, 1) codes: the sum of two uniforms. Its mean is zero and
// its variance does not depend on the signal, so no level is brightened or darkened.
float dither_tpdf(uvec2 p, uint channel) {
    return dither_hash(p, channel * 2u) + dither_hash(p, channel * 2u + 1u) - 1.0;
}

// `color` (0..1) plus that noise, in 1/255 steps. Within a code of black or white the
// noise shrinks to the distance left, so black stays black and white stays white.
vec3 dither8(vec3 color, uvec2 pixel) {
    vec3 out_c;
    for (uint c = 0u; c < 3u; c++) {
        float v = clamp(color[c], 0.0, 1.0) * 255.0;
        float room = min(min(v, 255.0 - v), 1.0);
        out_c[c] = (v + dither_tpdf(pixel, c) * room) / 255.0;
    }
    return out_c;
}
