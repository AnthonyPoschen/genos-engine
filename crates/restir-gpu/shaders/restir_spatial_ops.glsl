// One spatial pass: Chebyshev neighbours in row-major order, then the
// balance weight over that list. Included after restir_merge.glsl.
// History stays the post-temporal reservoir. The caller stores that.

#ifndef RESTIR_WIDTH
#define RESTIR_WIDTH pc.width
#endif
#ifndef RESTIR_SPATIAL_NEIGHBORS
#define RESTIR_SPATIAL_NEIGHBORS pc.spatial_neighbors
#endif
#ifndef RESTIR_SPATIAL_RADIUS
#define RESTIR_SPATIAL_RADIUS pc.spatial_radius
#endif
#ifndef RESTIR_REVERSE
#define RESTIR_REVERSE pc.reverse
#endif

float phat_at(uint recv, uint lamp, uint n) {
    uint limit = RESTIR_PROPOSAL >= PROPOSAL_TREE ? RESTIR_LAMP_COUNT : n;
    if (lamp == LAMP_EMPTY || lamp >= limit) {
        return 0.0;
    }
    return restir_phat(recv, lamp);
}

uint neighbor_count(uint index, uint receivers, out uint picked[16]) {
    uint width = RESTIR_WIDTH;
    if (width == 0u || RESTIR_SPATIAL_NEIGHBORS == 0u || RESTIR_SPATIAL_RADIUS == 0u) {
        return 0u;
    }
    uint height = receivers / width;
    uint x = index % width;
    uint y = index / width;
    uint reach = RESTIR_SPATIAL_RADIUS;
    uint x0 = x > reach ? x - reach : 0u;
    uint y0 = y > reach ? y - reach : 0u;
    uint x1 = min(x + reach, width - 1u);
    uint y1 = min(y + reach, height - 1u);
    uint pool[256];
    uint pool_n = 0u;
    for (uint row = y0; row <= y1; row++) {
        for (uint col = x0; col <= x1; col++) {
            if (col == x && row == y) {
                continue;
            }
            pool[pool_n] = row * width + col;
            pool_n += 1u;
        }
    }
    uint take = RESTIR_SPATIAL_NEIGHBORS;
    if (take > pool_n) {
        take = pool_n;
    }
    if (take > 16u) {
        take = 16u;
    }
    for (uint slot = 0u; slot < take; slot++) {
        uint span = pool_n - slot;
        uint pick = slot + uniform_index(span);
        uint tmp = pool[slot];
        pool[slot] = pool[pick];
        pool[pick] = tmp;
    }
    if (RESTIR_REVERSE != 0u) {
        for (uint i = 0u; i < take / 2u; i++) {
            uint j = take - 1u - i;
            uint tmp = pool[i];
            pool[i] = pool[j];
            pool[j] = tmp;
        }
    }
    for (uint i = 0u; i < take; i++) {
        picked[i] = pool[i];
    }
    return take;
}

void balance_list(
    uint nslots,
    uint src_recv[17],
    uint in_lamp[17],
    float in_w[17],
    uint in_m[17],
    float in_target[17],
    float in_tq[17],
    float unit,
    uint nlamps,
    out uint lamp,
    out float w_sum,
    out uint m,
    out float target
) {
    if (nslots == 2u && is_empty(in_lamp[0], in_m[0])) {
        if (is_empty(in_lamp[1], in_m[1])) {
            store_empty(lamp, w_sum, m, target);
        } else {
            lamp = in_lamp[1];
            w_sum = in_w[1];
            m = in_m[1];
            target = in_target[1];
        }
        return;
    }
    float mass[17];
    float area = 0.0;
    for (uint i = 0u; i < nslots; i++) {
        float weight = 0.0;
        if (!is_empty(in_lamp[i], in_m[i]) && in_tq[i] > 0.0) {
            weight = m_as_f(in_m[i]) * reservoir_w(in_lamp[i], in_w[i], in_m[i], in_target[i]) * in_tq[i];
        }
        mass[i] = weight;
        area += weight;
    }
    if (area <= 0.0) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    int selected = -1;
    if (nslots == 2u) {
        if (mass[1] > 0.0 && unit < mass[1] / area) {
            selected = 1;
        } else if (mass[0] > 0.0) {
            selected = 0;
        } else if (mass[1] > 0.0) {
            selected = 1;
        }
    } else {
        float mark = unit * area;
        float acc = 0.0;
        int last = -1;
        for (uint i = 0u; i < nslots; i++) {
            if (mass[i] <= 0.0) {
                continue;
            }
            last = int(i);
            acc += mass[i];
            if (mark < acc) {
                selected = int(i);
                break;
            }
        }
        if (selected < 0) {
            selected = last;
        }
    }
    if (selected < 0) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    uint sel = uint(selected);
    uint sel_lamp = in_lamp[sel];
    float tq = max(in_tq[sel], 0.0);
    float t_src = max(phat_at(src_recv[sel], sel_lamp, nlamps), 0.0);
    if (tq <= 0.0 || t_src <= 0.0) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    float denom = 0.0;
    uint count = 0u;
    for (uint i = 0u; i < nslots; i++) {
        if (is_empty(in_lamp[i], in_m[i])) {
            continue;
        }
        count += in_m[i];
        denom += m_as_f(in_m[i]) * max(phat_at(src_recv[i], sel_lamp, nlamps), 0.0);
    }
    if (denom <= 0.0 || count == 0u) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    float w_new = area * t_src / (tq * denom);
    lamp = sel_lamp;
    w_sum = w_new * m_as_f(count) * tq;
    m = count;
    target = tq;
}
