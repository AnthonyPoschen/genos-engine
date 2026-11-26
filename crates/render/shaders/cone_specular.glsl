// Cone-traced cascades: a metal's specular light (key cone_cascades.specular).
// The G-buffer's diffuse albedo is base colour times (1 - metallic), so a
// full metal takes no diffuse light; what it shows is what it reflects.
// Three parts, each times the pixel's metallic:
//   - the sun's and the lamps' highlights: glTF 2.0 Appendix B's metal lobe
//     F(base) D V (pbr_brdf.glsl) on the light the pixel already takes (its
//     shadow share is the diffuse light's);
//   - the specular cone (cone_specular.comp): one cone through the field
//     along the mirror direction, as wide as the GGX lobe, for a roughness
//     under CONE_SPECULAR_ROUGH (ADR 0019: one direction a pixel, a cone
//     read of finished stored light, no radiance history);
//   - at and over CONE_SPECULAR_ROUGH, the probes' gathered light (ADR 0019,
//     reflections design "Fit": that band is the diffuse gather).
// The surroundings' light is weighted by the split-sum environment term
// (Karis, "Real Shading in Unreal Engine 4", SIGGRAPH 2013) in Karis's
// analytic fit ("Physically Based Shading on Mobile", 2014). The dielectric
// lobe (F0 0.04) is not drawn: a dielectric keeps its Lambert picture.
// Include after lighting_common.glsl and cone_common.glsl.
#ifndef CONE_SPECULAR_GLSL
#define CONE_SPECULAR_GLSL
#include "pbr_brdf.glsl"

// Push lamp.x bit 6: the specular light is on.
const uint CONE_SPECULAR_FLAG = 64u;
// Header word (cleared each frame): pixels listed for the specular pass.
const uint CONE_SPECULAR_COUNT = 53u;
// Header word of the specular list's offset: a pixel index a word.
const uint CONE_SPECULAR_LAYOUT = 104u;
// Under half of one G-buffer code of metallic a pixel has no metal.
const float CONE_SPECULAR_METAL_MIN = 0.5 / 255.0;
// At and over this roughness the specular light is the gathered light; the
// cone fades out over the band below it, so a roughness texture does not
// draw a line where it crosses.
const float CONE_SPECULAR_ROUGH = 0.6;
const float CONE_SPECULAR_ROUGH_BLEND = 0.45;
// The least roughness a highlight and a cone are drawn with: the field's
// voxels are 6.25 cm and a lamp is a sphere, so nothing sharper is there to
// reflect, and at roughness 0 the GGX term is a point.
const float CONE_SPECULAR_ROUGH_MIN = 0.1;
// The least cosine between the view and the normal: a normal map can turn a
// seen surface's normal just past the view.
const float CONE_SPECULAR_VIEW_MIN = 0.05;
// The least cosine between the cone and the surface it leaves.
const float CONE_SPECULAR_LEAVE_MIN = 0.05;
// The widest cone: half an angle of 45 degrees.
const float CONE_SPECULAR_HALF_MAX = 0.78539816;
// Karis's fit of the environment term.
const vec4 CONE_ENV_C0 = vec4(-1.0, -0.0275, -0.572, 0.022);
const vec4 CONE_ENV_C1 = vec4(1.0, 0.0425, 1.04, -0.04);
const float CONE_ENV_EXP = -9.28;
const vec2 CONE_ENV_SCALE = vec2(-1.04, 1.04);
const float CONE_DIR_EPS = 1.0e-6;

bool cone_specular_on() {
    return (pc.lamp.x & CONE_SPECULAR_FLAG) != 0u;
}
// The view that draws the specular light alone (cone_cascades.view=specular).
bool cone_specular_view() {
    return ((pc.lamp.x >> CONE_VIEW_SHIFT) & 3u) == CONE_VIEW_SPECULAR;
}
uint cone_specular_at() {
    return cwork.w[CONE_SPECULAR_LAYOUT];
}

// A pixel's metal: its base colour, roughness and metallic, the way to the
// eye, and its normal turned to the eye's side when it is two-sided.
struct ConeMetal {
    vec3 base;
    float rough;
    float metal;
    vec3 view;
    vec3 n;
};

// False when the pixel has no metal or the specular light is off.
bool cone_metal(uvec2 p, vec3 n, bool two_sided, out ConeMetal m) {
    m.view = -lighting_eye_dir(p);
    m.n = two_sided && dot(n, m.view) < 0.0 ? -n : n;
    gbuf_material(p.y * pc.dims.x + p.x, m.base, m.rough, m.metal);
    m.rough = max(m.rough, CONE_SPECULAR_ROUGH_MIN);
    return cone_specular_on() && m.metal > CONE_SPECULAR_METAL_MIN;
}

// The share of the surroundings' specular light the cone brings; the
// gathered light brings the rest.
float cone_specular_share(float rough) {
    return 1.0 - smoothstep(CONE_SPECULAR_ROUGH_BLEND, CONE_SPECULAR_ROUGH, rough);
}

// The metal's highlight of light `e` (irradiance on the pixel, its n.l and
// shadow share in it) that arrives from `toward`.
vec3 cone_metal_direct(ConeMetal m, vec3 toward, vec3 e) {
    float ndl = dot(m.n, toward);
    if (ndl <= 0.0) {
        return vec3(0.0);
    }
    float ndv = max(dot(m.n, m.view), CONE_SPECULAR_VIEW_MIN);
    vec3 halfv = toward + m.view;
    float hlen = length(halfv);
    halfv = hlen > CONE_DIR_EPS ? halfv / hlen : m.n;
    vec3 cx = cross(m.n, halfv);
    return m.metal * pbr_eval(m.base, 1.0, m.rough, ndl, ndv, dot(m.n, halfv), dot(cx, cx), dot(m.view, halfv)) * e;
}

// The first sun's direction (toward it) at `from`; false with no sun.
bool cone_sun_toward(vec3 from, out vec3 toward) {
    toward = vec3(0.0);
    LampList list = lamps_at(from);
    for (uint k = 0u; k < list.suns.y; k++) {
        Lamp lamp = lamp_list_get(list, k);
        if (lamp.pos.w > CONE_SUN_MARK) {
            toward = -normalize(lamp.pos.xyz);
            return true;
        }
    }
    return false;
}

// The metal's weight on the radiance of its surroundings: metallic times
// the environment term with F0 the base colour.
vec3 cone_metal_env(ConeMetal m) {
    float ndv = max(dot(m.n, m.view), CONE_SPECULAR_VIEW_MIN);
    vec4 r = m.rough * CONE_ENV_C0 + CONE_ENV_C1;
    float a004 = min(r.x * r.x, exp2(CONE_ENV_EXP * ndv)) * r.x + r.y;
    vec2 ab = CONE_ENV_SCALE * a004 + r.zw;
    return m.metal * (m.base * ab.x + ab.y);
}

// The rough band's specular light: the gathered irradiance as the mean
// radiance of the surroundings, the share the cone does not bring.
vec3 cone_metal_gathered(ConeMetal m, vec3 irradiance) {
    return cone_metal_env(m) * (1.0 - cone_specular_share(m.rough)) * irradiance * (1.0 / CONE_PI);
}

// The specular cone's direction and its width a metre out.
void cone_specular_cone(ConeMetal m, out vec3 dir, out float width) {
    dir = reflect(-m.view, m.n);
    float leave = dot(dir, m.n);
    if (leave < CONE_SPECULAR_LEAVE_MIN) {
        dir = normalize(dir + m.n * (CONE_SPECULAR_LEAVE_MIN - leave));
    }
    // The GGX half vector's angle at half its distribution is atan(alpha);
    // the mirrored direction turns by twice that.
    float half_angle = min(CONE_HALVES * atan(pbr_alpha(m.rough)), CONE_SPECULAR_HALF_MAX);
    width = CONE_HALVES * tan(half_angle);
}

#endif
