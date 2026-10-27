#version 450
#extension GL_GOOGLE_include_directive : require
#include "reflections.glsl"

layout(location = 0) in vec3 in_pos;
layout(location = 1) in vec3 in_albedo;
layout(location = 2) in vec3 in_normal;
layout(location = 3) in float in_shade;
layout(location = 4) in vec2 in_uv;
layout(location = 0) out vec3 v_pos;
layout(location = 1) out vec3 v_albedo;
layout(location = 2) out vec3 v_normal;
layout(location = 3) out float v_shade;
layout(location = 4) out vec2 v_uv;

struct Link {
    vec4 eye;
    vec4 forward;
    vec4 origin;
    vec4 normal;
    vec4 to_origin;
    vec4 from0;
    vec4 from1;
    vec4 from2;
    vec4 to0;
    vec4 to1;
    vec4 to2;
    vec4 right;
    vec4 up;
};

layout(std430, set = 0, binding = 0) readonly buffer Links {
    Link link;
} links;

struct Instance {
    mat4 model;
    vec4 color;
    vec4 atlas;
    vec4 normal_atlas;
    vec4 extra;
};

layout(std430, set = 0, binding = 1) readonly buffer Instances {
    Instance items[];
} instances;

vec3 portal_of(vec3 point, mat3 from_basis, mat3 to_basis) {
    Link link = links.link;
    return reflection_portal_point(point, link.origin.xyz, from_basis, link.to_origin.xyz, to_basis);
}

void main() {
    Link link = links.link;
    Instance item = instances.items[gl_InstanceIndex];
    vec4 world = item.model * vec4(in_pos, 1.0);
    mat3 from_basis = mat3(link.from0.xyz, link.from1.xyz, link.from2.xyz);
    mat3 to_basis = mat3(link.to0.xyz, link.to1.xyz, link.to2.xyz);
    vec3 eye = portal_of(link.eye.xyz, from_basis, to_basis);
    vec3 forward = normalize(portal_of(link.eye.xyz + link.forward.xyz, from_basis, to_basis) - eye);
    // The portal image of the camera axes. Rebuilding them from world up
    // drops the roll of a pitched destination basis, so the opening no longer
    // lines up with the screen.
    vec3 right = normalize(portal_of(link.eye.xyz + link.right.xyz, from_basis, to_basis) - eye);
    vec3 up = normalize(portal_of(link.eye.xyz + link.up.xyz, from_basis, to_basis) - eye);
    vec3 destination = portal_of(link.origin.xyz, from_basis, to_basis);
    vec3 plane_n = normalize(portal_of(link.origin.xyz + link.normal.xyz, from_basis, to_basis) - destination);
    float aspect = max(link.normal.w, GRADE_ASPECT_EPS);
    float h = 1.0 / tan(radians(GRADE_FOV_DEG) * GRADE_HALF);
    float w = h / aspect;
    const float near_z = GRADE_NEAR;
    const float far_z = GRADE_FAR;
    mat4 proj = mat4(0.0);
    proj[0][0] = w;
    proj[1][1] = -h;
    proj[2][2] = far_z / (near_z - far_z);
    proj[2][3] = -1.0;
    proj[3][2] = (far_z * near_z) / (near_z - far_z);
    vec3 rel = destination - eye;
    vec3 origin_cam = vec3(dot(right, rel), dot(up, rel), -dot(forward, rel));
    vec3 normal_cam = vec3(dot(right, plane_n), dot(up, plane_n), -dot(forward, plane_n));
    vec4 plane = vec4(normal_cam, -dot(normal_cam, origin_cam));
    vec4 row = reflection_oblique_row(proj, plane);
    proj[0][2] = row.x;
    proj[1][2] = row.y;
    proj[2][2] = row.z;
    proj[3][2] = row.w;
    mat4 view = mat4(
        vec4(right.x, up.x, -forward.x, 0.0),
        vec4(right.y, up.y, -forward.y, 0.0),
        vec4(right.z, up.z, -forward.z, 0.0),
        vec4(-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0)
    );
    vec3 spun = mat3(item.model) * in_normal;
    float len = length(spun);
    gl_Position = proj * view * world;
    v_pos = world.xyz;
    v_albedo = in_albedo * item.color.rgb;
    v_normal = len > 1e-6 ? spun / len : spun;
    v_shade = in_shade;
    v_uv = in_uv;
}
