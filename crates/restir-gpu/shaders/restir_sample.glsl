// Initial candidates (Talbot, Cline, and Egbert, EGSR 2005).
// Included after target_at (the checker) or restir_phat (the live pass),
// sample_tree, next_f32, and the proposal constants.
// The checker leaves the macros on the push constant. The live pass sets them
// to header words before the include.

#ifndef RESTIR_CANDIDATES
#define RESTIR_CANDIDATES pc.candidates
#endif
#ifndef RESTIR_LAMP_COUNT
#define RESTIR_LAMP_COUNT pc.lamp_count
#endif
#ifndef RESTIR_PROPOSAL
#define RESTIR_PROPOSAL pc.proposal
#endif

#ifndef RESTIR_LIVE_PHAT
float restir_phat(uint recv, uint lamp) {
    return target_at(recv, lamp);
}
#endif

uint uniform_index(uint n) {
    if (n <= 1u) {
        return 0u;
    }
    uint limit = 0xffffffffu - (0xffffffffu % n);
    while (true) {
        uint draw = next_u32();
        if (draw < limit) {
            return draw % n;
        }
    }
}

void sample_cdf(const float weights[64], uint n, out uint index, out float pdf, out bool hit) {
    float sum = 0.0;
    for (uint i = 0u; i < n; i++) {
        sum += weights[i];
    }
    if (sum <= 0.0) {
        next_f32();
        hit = false;
        index = 0u;
        pdf = 0.0;
        return;
    }
    float mark = next_f32() * sum;
    float acc = 0.0;
    for (uint i = 0u; i < n; i++) {
        if (weights[i] <= 0.0) {
            continue;
        }
        acc += weights[i];
        if (mark < acc) {
            index = i;
            pdf = weights[i] / sum;
            hit = true;
            return;
        }
    }
    for (uint k = n; k > 0u; k--) {
        uint i = k - 1u;
        if (weights[i] > 0.0) {
            index = i;
            pdf = weights[i] / sum;
            hit = true;
            return;
        }
    }
    hit = false;
    index = 0u;
    pdf = 0.0;
}

void stream_update(inout uint lamp, inout float w_sum, inout uint m, uint next_lamp, float stream) {
    float unit = next_f32();
    m += 1u;
    if (stream > 0.0) {
        w_sum += stream;
        if (unit * w_sum < stream) {
            lamp = next_lamp;
        }
    }
}

bool is_empty(uint lamp, uint m) {
    return m == 0u || lamp == LAMP_EMPTY;
}

void store_empty(out uint lamp, out float w_sum, out uint m, out float target) {
    lamp = LAMP_EMPTY;
    w_sum = 0.0;
    m = 0u;
    target = 0.0;
}

void initial_at(uint recv, uint n, out uint lamp, out float w_sum, out uint m, out float target, out float weights[64]) {
    bool tree = RESTIR_PROPOSAL >= PROPOSAL_TREE;
    if (!tree) {
        for (uint i = 0u; i < n; i++) {
            weights[i] = restir_phat(recv, i);
        }
    }
    lamp = LAMP_EMPTY;
    w_sum = 0.0;
    m = 0u;
    for (uint c = 0u; c < RESTIR_CANDIDATES; c++) {
        bool hit = false;
        uint next_lamp = 0u;
        float pdf = 0.0;
        if (tree) {
            sample_tree(recv, next_lamp, pdf, hit);
        } else if (RESTIR_PROPOSAL == PROPOSAL_TARGET) {
            sample_cdf(weights, n, next_lamp, pdf, hit);
        } else if (n == 0u) {
            hit = false;
        } else {
            next_lamp = uniform_index(n);
            pdf = 1.0 / float(n);
            hit = true;
        }
        if (!hit) {
            stream_update(lamp, w_sum, m, LAMP_EMPTY, 0.0);
        } else {
            float phat = tree ? restir_phat(recv, next_lamp) : weights[next_lamp];
            float stream = pdf > 0.0 ? phat / pdf : 0.0;
            stream_update(lamp, w_sum, m, next_lamp, stream);
        }
    }
    target = 0.0;
    if (lamp != LAMP_EMPTY && (tree ? lamp < RESTIR_LAMP_COUNT : lamp < n)) {
        target = tree ? restir_phat(recv, lamp) : weights[lamp];
    }
    if (m == 0u || lamp == LAMP_EMPTY || target <= 0.0 || w_sum <= 0.0) {
        store_empty(lamp, w_sum, m, target);
    }
}
