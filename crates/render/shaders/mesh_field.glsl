// Mesh SDF instances for the GI v2 software tracer (mesh_field.rs packs the words).
//
// Each traced mesh instance is a sparse signed distance field in object space:
// 8^3 bricks of byte distances near the surface and one coarse byte per brick
// everywhere else. A ray is clipped to the instance's world bounds, moved into
// object space, and sphere-traced: inside a stored brick by the trilinear fine
// distance, elsewhere by the coarse value less the distance to its brick centre
// (never less than the band, since a brick away from the surface stores nothing).
//
// Needs SceneHit and SCENE_LAMBERT declared first (scene_rays.glsl has them).
// Include it only in the small GI v2 passes, never in scene.frag or light.comp:
// inlined at their many trace sites it stalled the NVIDIA compiler (4070, over 4
// minutes at pipeline creation) and ran lavapipe out of memory.
//
// Loops run over data bounds (instance count and step cap from the header) with
// [[dont_unroll]], as everywhere in scene_rays.glsl.

layout(std430, set = 0, binding = 5) readonly buffer MeshField {
    uint words[];
} mesh_field;

const uint MF_HEADER = 4u;
const uint MF_RECORD = 32u;

struct MfInst {
    vec4 r0;
    vec4 r1;
    vec4 r2;
    vec3 lo;
    float dscale;
    vec3 hi;
    uint albedo;
    vec3 origin;
    float voxel;
    uvec3 dims;
    uint table;
    float band;
    float coarse_band;
    uint coarse;
    bool thin;
};

uint mf_count() {
    return mesh_field.words[0];
}

float mf_f(uint w) {
    return uintBitsToFloat(mesh_field.words[w]);
}

MfInst mf_inst(uint i) {
    uint b = MF_HEADER + i * MF_RECORD;
    MfInst m;
    m.r0 = vec4(mf_f(b), mf_f(b + 1u), mf_f(b + 2u), mf_f(b + 3u));
    m.r1 = vec4(mf_f(b + 4u), mf_f(b + 5u), mf_f(b + 6u), mf_f(b + 7u));
    m.r2 = vec4(mf_f(b + 8u), mf_f(b + 9u), mf_f(b + 10u), mf_f(b + 11u));
    m.lo = vec3(mf_f(b + 12u), mf_f(b + 13u), mf_f(b + 14u));
    m.dscale = mf_f(b + 15u);
    m.hi = vec3(mf_f(b + 16u), mf_f(b + 17u), mf_f(b + 18u));
    m.albedo = mesh_field.words[b + 19u];
    m.origin = vec3(mf_f(b + 20u), mf_f(b + 21u), mf_f(b + 22u));
    m.voxel = mf_f(b + 23u);
    m.dims = uvec3(mesh_field.words[b + 24u], mesh_field.words[b + 25u], mesh_field.words[b + 26u]);
    m.table = mesh_field.words[b + 27u];
    m.band = mf_f(b + 28u);
    m.coarse_band = mf_f(b + 29u);
    m.coarse = mesh_field.words[b + 30u];
    m.thin = mesh_field.words[b + 31u] != 0u;
    return m;
}

float mf_byte(uint at, uint index) {
    uint w = mesh_field.words[at + (index >> 2u)];
    return float((w >> ((index & 3u) * 8u)) & 255u);
}

float mf_decode(float b, float band) {
    return (b / 255.0 * 2.0 - 1.0) * band;
}

uint mf_cell(MfInst m, uvec3 b) {
    return (b.z * m.dims.y + b.y) * m.dims.x + b.x;
}

// One voxel's stored distance; a voxel off the grid or in an empty brick is at least
// the band away. Branch-free (selects only): this is inlined at every trace site, and
// a block per test multiplies the driver's compile time.
float mf_voxel(MfInst m, ivec3 v) {
    ivec3 size = ivec3(m.dims) * 8;
    bool on = all(greaterThanEqual(v, ivec3(0))) && all(lessThan(v, size));
    uvec3 u = uvec3(clamp(v, ivec3(0), size - 1));
    uint at = mesh_field.words[m.table + mf_cell(m, u >> 3u)];
    uvec3 l = u & 7u;
    float d = mf_decode(mf_byte(max(at, 1u) - 1u, (l.z * 8u + l.y) * 8u + l.x), m.band);
    return (on && at != 0u) ? d : m.band;
}

// Trilinear distance over voxel centres at grid coordinate g (voxels), with its
// gradient (per voxel) in yzw: one set of eight reads gives both.
vec4 mf_fine(MfInst m, vec3 g) {
    vec3 q = g - 0.5;
    ivec3 v = ivec3(floor(q));
    vec3 f = q - vec3(v);
    float c0 = mf_voxel(m, v);
    float c1 = mf_voxel(m, v + ivec3(1, 0, 0));
    float c2 = mf_voxel(m, v + ivec3(0, 1, 0));
    float c3 = mf_voxel(m, v + ivec3(1, 1, 0));
    float c4 = mf_voxel(m, v + ivec3(0, 0, 1));
    float c5 = mf_voxel(m, v + ivec3(1, 0, 1));
    float c6 = mf_voxel(m, v + ivec3(0, 1, 1));
    float c7 = mf_voxel(m, v + ivec3(1, 1, 1));
    float x00 = mix(c0, c1, f.x);
    float x10 = mix(c2, c3, f.x);
    float x01 = mix(c4, c5, f.x);
    float x11 = mix(c6, c7, f.x);
    float y0 = mix(x00, x10, f.y);
    float y1 = mix(x01, x11, f.y);
    vec3 grad = vec3(
        mix(mix(c1 - c0, c3 - c2, f.y), mix(c5 - c4, c7 - c6, f.y), f.z),
        mix(x10 - x00, x11 - x01, f.z),
        y1 - y0);
    return vec4(mix(y0, y1, f.z), grad);
}

// Object-space distance at p, safe for sphere tracing (never more than the truth
// outside the band; the fine value inside it), and a gradient (zero off the band).
vec4 mf_sample(MfInst m, vec3 p) {
    vec3 g = (p - m.origin) / m.voxel;
    vec3 bf = clamp(floor(g / 8.0), vec3(0.0), vec3(m.dims) - 1.0);
    uint cell = mf_cell(m, uvec3(bf));
    vec4 fine = mf_fine(m, g);
    float c = mf_decode(mf_byte(m.coarse, cell), m.coarse_band);
    float off = length(g - (bf + 0.5) * 8.0) * m.voxel;
    float coarse = c >= 0.0 ? max(c - off, m.band) : -max(-c - off, m.band);
    return mesh_field.words[m.table + cell] != 0u ? fine : vec4(coarse, vec3(0.0));
}

vec3 mf_to_object(MfInst m, vec3 p) {
    vec4 q = vec4(p, 1.0);
    return vec3(dot(m.r0, q), dot(m.r1, q), dot(m.r2, q));
}

vec3 mf_dir_to_object(MfInst m, vec3 d) {
    return vec3(dot(m.r0.xyz, d), dot(m.r1.xyz, d), dot(m.r2.xyz, d));
}

bool mf_slab(vec3 o, vec3 inv, vec3 lo, vec3 hi, inout float ta, inout float tb) {
    vec3 a = (lo - o) * inv;
    vec3 b = (hi - o) * inv;
    vec3 n = min(a, b);
    vec3 f = max(a, b);
    ta = max(ta, max(n.x, max(n.y, n.z)));
    tb = min(tb, min(f.x, min(f.y, f.z)));
    return ta <= tb;
}

// Surface distance under which a march counts as a hit (object metres).
float mf_eps(MfInst m) {
    return m.thin ? 0.5 * m.voxel : 0.2 * m.voxel;
}

// Nearest hit of instance i with t0 <= t < hit.t (world metres); updates hit.
bool mf_trace_one(uint i, vec3 origin, vec3 dir, float t0, inout SceneHit hit) {
    MfInst m = mf_inst(i);
    vec3 inv = 1.0 / max(abs(dir), vec3(1.0e-12)) * sign(dir + 1.0e-30);
    float wa = t0;
    float wb = hit.t;
    if (!mf_slab(origin, inv, m.lo, m.hi, wa, wb)) {
        return false;
    }
    vec3 o = mf_to_object(m, origin);
    vec3 d = mf_dir_to_object(m, dir);
    float s = length(d);
    if (s < 1.0e-12) {
        return false;
    }
    d /= s;
    float ta = wa * s;
    float tb = wb * s;
    vec3 grid_hi = m.origin + vec3(m.dims) * 8.0 * m.voxel;
    vec3 dinv = 1.0 / max(abs(d), vec3(1.0e-12)) * sign(d + 1.0e-30);
    if (!mf_slab(o, dinv, m.origin, grid_hi, ta, tb)) {
        return false;
    }
    float eps = mf_eps(m);
    float t = ta;
    // A ray that starts on this mesh (a shadow or bounce ray from its own surface)
    // and leaves it skips past its own skin once; one that heads into it is blocked.
    bool start = ta <= t0 * s + 1.0e-6;
    uint steps = mesh_field.words[1];
    [[dont_unroll]] for (uint k = 0u; k < steps && t < tb; k++) {
        vec4 dg = mf_sample(m, o + d * t);
        if (dg.x < eps) {
            if (start && dg.x > -2.0 * eps && (m.thin || dot(dg.yzw, d) >= 0.0)) {
                start = false;
                t += 3.0 * eps;
                continue;
            }
            vec3 nw = dg.y * m.r0.xyz + dg.z * m.r1.xyz + dg.w * m.r2.xyz;
            float len = length(nw);
            nw = len > 1.0e-12 ? nw / len : -dir;
            if (m.thin && dot(nw, dir) > 0.0) {
                nw = -nw;
            }
            hit.t = t / s;
            hit.normal = nw;
            hit.albedo = unpackUnorm4x8(m.albedo).rgb;
            hit.reflect = SCENE_LAMBERT;
            hit.color_mix = 1.0;
            return true;
        }
        start = false;
        t += dg.x;
    }
    return false;
}

bool mf_trace(vec3 origin, vec3 dir, float t0, inout SceneHit hit) {
    bool found = false;
    uint n = mf_count();
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        if (mf_trace_one(i, origin, dir, t0, hit)) {
            found = true;
        }
    }
    return found;
}

// Strictly inside a closed traced mesh.
bool mf_inside(vec3 p) {
    uint n = mf_count();
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        MfInst m = mf_inst(i);
        if (m.thin || any(lessThan(p, m.lo)) || any(greaterThan(p, m.hi))) {
            continue;
        }
        if (mf_sample(m, mf_to_object(m, p)).x < -mf_eps(m)) {
            return true;
        }
    }
    return false;
}

// Any traced instance whose world bounds meet lo..hi.
bool mf_meets(vec3 lo, vec3 hi) {
    uint n = mf_count();
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        uint b = MF_HEADER + i * MF_RECORD;
        vec3 ilo = vec3(mf_f(b + 12u), mf_f(b + 13u), mf_f(b + 14u));
        vec3 ihi = vec3(mf_f(b + 16u), mf_f(b + 17u), mf_f(b + 18u));
        if (all(lessThanEqual(ilo, hi)) && all(lessThanEqual(lo, ihi))) {
            return true;
        }
    }
    return false;
}
