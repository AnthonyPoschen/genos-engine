// Light-tree walk. Included after load_f, next_f32, and the push constant `pc`.
// Arithmetic matches crates/restir-ref/src/tree.rs. One f32 per internal node.
// A root leaf consumes none. proposal 2 is the bound. proposal 3 drops 1/d^2.

const uint PROPOSAL_TREE = 2u;
const uint PROPOSAL_TREE_FAULT = 3u;
const uint NODE_STRIDE = 12u;
const uint NODE_LAMP_EMPTY = 0xffffffffu;
const float TREE_MIN_DIST = 1.0e-3;
const float HALF = 0.5;

float tree_sqrt(float value) {
    if (!(value > 0.0)) {
        return 0.0;
    }
    uint bits = floatBitsToUint(value);
    uint guess = (bits >> 1u) + (127u << 22u);
    float y = uintBitsToFloat(guess);
    y = HALF * (y + value / y);
    y = HALF * (y + value / y);
    y = HALF * (y + value / y);
    y = HALF * (y + value / y);
    if (y == y && y != 1.0 / 0.0 && y != -1.0 / 0.0) {
        return y;
    }
    return 0.0;
}

float tree_dot(float ax, float ay, float az, float bx, float by, float bz) {
    return 0.0 + ax * bx + ay * by + az * bz;
}

float node_importance(uint node, uint recv, bool ignore_distance) {
    uint base = pc.tree_base + node * NODE_STRIDE;
    float power = load_f(base + 3u);
    if (power <= 0.0) {
        return 0.0;
    }
    float min_x = load_f(base);
    float min_y = load_f(base + 1u);
    float min_z = load_f(base + 2u);
    float max_x = load_f(base + 4u);
    float max_y = load_f(base + 5u);
    float max_z = load_f(base + 6u);
    float radius = load_f(base + 7u);
    float center_x = HALF * (min_x + max_x);
    float center_y = HALF * (min_y + max_y);
    float center_z = HALF * (min_z + max_z);
    uint r = pc.recv_base + recv * 9u;
    float px = load_f(r);
    float py = load_f(r + 1u);
    float pz = load_f(r + 2u);
    float dx = center_x - px;
    float dy = center_y - py;
    float dz = center_z - pz;
    float dist2 = tree_dot(dx, dy, dz, dx, dy, dz);
    float dist = tree_sqrt(dist2);
    float box_dx = 0.0;
    float box_dy = 0.0;
    float box_dz = 0.0;
    if (px < min_x) {
        box_dx = min_x - px;
    } else if (px > max_x) {
        box_dx = px - max_x;
    }
    if (py < min_y) {
        box_dy = min_y - py;
    } else if (py > max_y) {
        box_dy = py - max_y;
    }
    if (pz < min_z) {
        box_dz = min_z - pz;
    } else if (pz > max_z) {
        box_dz = pz - max_z;
    }
    float box_dist2 = tree_dot(box_dx, box_dy, box_dz, box_dx, box_dy, box_dz);
    float floor2 = TREE_MIN_DIST * TREE_MIN_DIST;
    if (box_dist2 < floor2) {
        box_dist2 = floor2;
    }
    float geom = ignore_distance ? 1.0 : 1.0 / box_dist2;
    float radius2 = radius * radius;
    if (dist2 <= radius2) {
        return power * geom;
    }
    float inv = 1.0 / dist;
    float cos_alpha = tree_dot(
        load_f(r + 3u), load_f(r + 4u), load_f(r + 5u),
        dx * inv, dy * inv, dz * inv);
    float sin_theta = radius * inv;
    if (sin_theta > 1.0) {
        sin_theta = 1.0;
    }
    float cos_theta = tree_sqrt(1.0 - sin_theta * sin_theta);
    float sin_alpha = tree_sqrt(1.0 - cos_alpha * cos_alpha);
    float cos_bound = cos_alpha * cos_theta + sin_alpha * sin_theta;
    if (cos_bound <= 0.0) {
        return 0.0;
    }
    return power * cos_bound * geom;
}

void sample_tree(uint recv, out uint index, out float pdf, out bool hit) {
    if (pc.tree_count == 0u) {
        hit = false;
        index = 0u;
        pdf = 0.0;
        return;
    }
    bool fault = pc.proposal == PROPOSAL_TREE_FAULT;
    uint node = 0u;
    float acc_pdf = 1.0;
    for (uint step = 0u; step < 64u; step++) {
        uint base = pc.tree_base + node * NODE_STRIDE;
        uint left = words[base + 8u];
        uint right = words[base + 9u];
        uint lamp = words[base + 10u];
        if (lamp != NODE_LAMP_EMPTY) {
            index = lamp;
            pdf = acc_pdf;
            hit = true;
            return;
        }
        if (left >= pc.tree_count || right >= pc.tree_count) {
            next_f32();
            hit = false;
            index = 0u;
            pdf = 0.0;
            return;
        }
        float left_i = node_importance(left, recv, fault);
        float right_i = node_importance(right, recv, fault);
        float sum = left_i + right_i;
        if (sum <= 0.0) {
            next_f32();
            hit = false;
            index = 0u;
            pdf = 0.0;
            return;
        }
        float mark = next_f32() * sum;
        if (right_i <= 0.0 || mark < left_i) {
            acc_pdf *= left_i / sum;
            node = left;
        } else {
            acc_pdf *= right_i / sum;
            node = right;
        }
    }
    hit = false;
    index = 0u;
    pdf = 0.0;
}
