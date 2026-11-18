// Shared Instance SSBO record. Must match `InstanceRec` in `gpu.rs` (std430).
// Every vertex shader that reads binding instances must include this file and
// must not declare its own `struct Instance`.
//
// Layout (144 B):
//   mat4  model         @  0  (64)
//   vec4  color         @ 64  (16)  — w holds emit bits as float
//   vec4  atlas         @ 80  (16)
//   vec4  normal_atlas  @ 96  (16)
//   vec4  extra         @112  (16)  — cutoff, roughness, metallic, flags
//   uvec4 ids           @128  (16)  — x = object id, y = bone id
struct Instance {
    mat4 model;
    vec4 color;
    vec4 atlas;
    vec4 normal_atlas;
    vec4 extra;
    uvec4 ids;
};
