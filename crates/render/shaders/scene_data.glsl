// The resident scene block every pass reads (pack.rs scene_bytes writes it).
//
// Fixed fields first, then a tail of 16-byte words that holds the lamps, the
// occluders and the grids over them. The lamps and occluders are read through
// scene_lamp and scene_occ; the grids through the helpers in scene_rays.glsl.

struct Lamp {
    vec4 pos;
    // rgb is the colour. w is the range in metres past which the lamp adds nothing
    // (pack.rs lamp_range); 0 for a sun, which reaches everywhere.
    vec4 color;
};
struct Occ {
    vec4 center_shape;
    vec4 extent;
    vec4 albedo;
    // x is reflectance, an albedo 0..1 (divided by pi where it shades). y is the
    // surface-color mix. Below zero means the game default. z and w are the cosine
    // and sine of the turn about +Y (a square solid's yaw; 1 and 0 for no turn).
    vec4 bounce;
};
struct Puff {
    vec4 center_density;
    vec4 radius;
};
struct Cascade {
    float spacing;
    float t0;
    float t1;
    float origin_x;
    float origin_z;
    float count_x;
    float count_z;
    float dirs;
    float offset;
    float pad0;
    float pad1;
    float pad2;
};
layout(std430, set = 0, binding = 0) readonly buffer SceneData {
    uint lamp_count;
    uint occ_count;
    uint obj_count;
    uint count_x;
    uint count_z;
    float spacing;
    float origin_x;
    float origin_z;
    float near_end;
    float far_end;
    float world_end;
    uint pad2;
    vec4 eye;
    vec4 fire_pos;
    vec4 fire_color;
    Puff puffs[8];
    // xyz is the floor center. w is half extent on X.
    vec4 floor_center;
    // x is half extent on Z. yzw is the floor color.
    vec4 floor_data;
    Cascade cascades[3];
    // xyz is the camera basis. w is tan(half fov), 0, aspect, and unused.
    vec4 view_right;
    vec4 view_up;
    vec4 view_forward;
    // x is the screen-probe columns. y is the rows. z is the near-field rays plus one
    // (0: NEAR_RAYS).
    vec4 view_grid;
    // x is the roof underside height over the floor footprint (0 = no roof). yzw is its color.
    vec4 ceiling;
    // rgb is the sky's radiance (0 = no sky): what a ray that leaves the scene brings.
    vec4 sky;
    // Occluder grid on the ground plane: low x, low z, cell size, top of the tallest
    // occluder.
    vec4 occ_grid;
    // Lamp grid: low x, low z, cell size, unused.
    vec4 lamp_grid;
    // Cells: occluder grid x and z, lamp grid x and z.
    uvec4 grid_dims;
    // Tail word where the lamps start (2 per lamp), the occluders (4 each), and the
    // grid words (4 uints per tail word); w is the sun count.
    uvec4 tail_at;
    // Grid word of the occluder cell table, the lamp cell table and the sun list.
    // A cell holds (first word, count) of its list of indices.
    uvec4 grid_at;
    uvec4 tail[];
} scene;

Lamp scene_lamp(uint i) {
    uint at = scene.tail_at.x + i * 2u;
    Lamp lamp;
    lamp.pos = uintBitsToFloat(scene.tail[at]);
    lamp.color = uintBitsToFloat(scene.tail[at + 1u]);
    return lamp;
}

Occ scene_occ(uint i) {
    uint at = scene.tail_at.y + i * 4u;
    Occ occ;
    occ.center_shape = uintBitsToFloat(scene.tail[at]);
    occ.extent = uintBitsToFloat(scene.tail[at + 1u]);
    occ.albedo = uintBitsToFloat(scene.tail[at + 2u]);
    occ.bounce = uintBitsToFloat(scene.tail[at + 3u]);
    return occ;
}

uint grid_word(uint u) {
    uvec4 v = scene.tail[scene.tail_at.z + (u >> 2u)];
    return v[u & 3u];
}
