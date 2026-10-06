#version 450
layout(location = 0) out vec4 out_color;
void main() {
    // 0.18 stays below 1 when coincident edges add.
    out_color = vec4(0.18, 0.18, 0.18, 1.0);
}
