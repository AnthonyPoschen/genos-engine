// Light-tree walk. Included after load_f, next_f32, and the push constant `pc`.
// Arithmetic matches crates/restir-ref/src/tree.rs. One f32 per internal node.
// A root leaf consumes none. proposal 2 is the bound. proposal 3 drops 1/d^2.

const uint PROPOSAL_TREE = 2u;
const uint PROPOSAL_TREE_FAULT = 3u;
const uint NODE_STRIDE = 12u;
const uint NODE_LAMP_EMPTY = 0xffffffffu;
// lamp_light's LAMP_RADIUS clamp; tree.rs MIN_DIST.
const float TREE_MIN_DIST = 0.1;
// restir_phat measures range from the lifted origin (tree.rs RANGE_SLACK).
const float TREE_RANGE_SLACK = 0.02;
const float HALF = 0.5;

// The checker reads these from the push constant. The live direct pass
// defines them as header words before this include.
#ifndef RESTIR_TREE_BASE
#define RESTIR_TREE_BASE pc.tree_base
#define RESTIR_RECV_BASE pc.recv_base
#define RESTIR_TREE_COUNT pc.tree_count
#define RESTIR_PROPOSAL pc.proposal
#endif
#ifndef RESTIR_WORDS
#define RESTIR_WORDS words
#endif

const float TREE_STEPS = 65536.0;

float tree_split_steps(float left_i, float right_i, float sum) {
    if (right_i <= 0.0) {
        return TREE_STEPS;
    }
    if (left_i <= 0.0) {
        return 0.0;
    }
    // A side with importance keeps at least one step, so no lamp loses support.
    return clamp(floor(left_i / sum * TREE_STEPS + 0.5), 1.0, TREE_STEPS - 1.0);
}

// Power-only share weight in each split's defensive mixture (tree.rs DEFENSIVE).
const float TREE_DEFENSIVE = 0.1;

// tree.rs split_steps_mixed: (1 - a) * bound share + a * power share.
float tree_split_steps_mixed(float left_i, float right_i, float left_p, float right_p) {
    if (right_i <= 0.0) {
        return TREE_STEPS;
    }
    if (left_i <= 0.0) {
        return 0.0;
    }
    float bound = left_i / (left_i + right_i);
    float power = left_p / (left_p + right_p);
    float share = (1.0 - TREE_DEFENSIVE) * bound + TREE_DEFENSIVE * power;
    return clamp(floor(share * TREE_STEPS + 0.5), 1.0, TREE_STEPS - 1.0);
}

#ifndef RESTIR_LIVE_RECEIVER
bool restir_receiver_two_sided(uint recv) {
    return false;
}

void restir_receiver(uint recv, out float px, out float py, out float pz, out float nx, out float ny, out float nz) {
    uint r = RESTIR_RECV_BASE + recv * 9u;
    px = load_f(r);
    py = load_f(r + 1u);
    pz = load_f(r + 2u);
    nx = load_f(r + 3u);
    ny = load_f(r + 4u);
    nz = load_f(r + 5u);
}
#endif

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
    uint base = RESTIR_TREE_BASE + node * NODE_STRIDE;
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
    float px;
    float py;
    float pz;
    float nx;
    float ny;
    float nz;
    restir_receiver(recv, px, py, pz, nx, ny, nz);
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
    float range = load_f(base + 11u);
    if (range > 0.0) {
        float reach = range + TREE_RANGE_SLACK;
        if (box_dist2 > reach * reach) {
            return 0.0;
        }
    }
    bool two = restir_receiver_two_sided(recv);
    if (!two) {
        float front = 0.0
            + nx * dx
            + ny * dy
            + nz * dz
            + abs(nx) * (HALF * (max_x - min_x))
            + abs(ny) * (HALF * (max_y - min_y))
            + abs(nz) * (HALF * (max_z - min_z));
        if (front <= 0.0) {
            return 0.0;
        }
    }
    float floor2 = TREE_MIN_DIST * TREE_MIN_DIST;
    if (box_dist2 < floor2) {
        box_dist2 = floor2;
    }
    float geom = ignore_distance ? 1.0 : 1.0 / box_dist2;
    // cos(max(alpha - theta_u, 0)), 1 inside the bounding sphere (tree.rs).
    float radius2 = radius * radius;
    float cos_bound = 1.0;
    if (dist2 > radius2) {
        float inv = 1.0 / dist;
        float cos_alpha = tree_dot(nx, ny, nz, dx * inv, dy * inv, dz * inv);
        if (two) {
            cos_alpha = abs(cos_alpha);
        }
        float sin_theta = radius * inv;
        if (sin_theta > 1.0) {
            sin_theta = 1.0;
        }
        float cos_theta = tree_sqrt(1.0 - sin_theta * sin_theta);
        if (cos_alpha < cos_theta) {
            float sin_alpha = tree_sqrt(1.0 - cos_alpha * cos_alpha);
            cos_bound = cos_alpha * cos_theta + sin_alpha * sin_theta;
        }
    }
    if (cos_bound <= 0.0) {
        return 0.0;
    }
    return power * cos_bound * geom;
}

void sample_tree(uint recv, out uint index, out float pdf, out bool hit) {
    if (RESTIR_TREE_COUNT == 0u) {
        hit = false;
        index = 0u;
        pdf = 0.0;
        return;
    }
    bool fault = RESTIR_PROPOSAL == PROPOSAL_TREE_FAULT;
    uint node = 0u;
    float acc_pdf = 1.0;
    for (uint step = 0u; step < 64u; step++) {
        uint base = RESTIR_TREE_BASE + node * NODE_STRIDE;
        uint left = RESTIR_WORDS[base + 8u];
        uint right = RESTIR_WORDS[base + 9u];
        uint lamp = RESTIR_WORDS[base + 10u];
        if (lamp != NODE_LAMP_EMPTY) {
            index = lamp;
            pdf = acc_pdf;
            hit = true;
            return;
        }
        if (left >= RESTIR_TREE_COUNT || right >= RESTIR_TREE_COUNT) {
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
        // next_f32 has 65536 steps: quantize the left share to them and charge
        // that share, so the pdf is the real pick probability (tree.rs split_steps_mixed).
        float left_p = load_f(RESTIR_TREE_BASE + left * NODE_STRIDE + 3u);
        float right_p = load_f(RESTIR_TREE_BASE + right * NODE_STRIDE + 3u);
        float left_q = tree_split_steps_mixed(left_i, right_i, left_p, right_p);
        float draw = next_f32() * TREE_STEPS;
        if (draw < left_q) {
            acc_pdf *= left_q / TREE_STEPS;
            node = left;
        } else {
            acc_pdf *= (TREE_STEPS - left_q) / TREE_STEPS;
            node = right;
        }
    }
    hit = false;
    index = 0u;
    pdf = 0.0;
}
