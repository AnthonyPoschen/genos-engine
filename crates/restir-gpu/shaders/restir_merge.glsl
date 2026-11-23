// Balance-heuristic merge (Bitterli et al., SIGGRAPH 2022, GRIS equation 22)
// and the history-M clamp (Bitterli et al., TOG 2020). Included after
// restir_sample.glsl. An empty canonical sample returns history unchanged.

#ifndef RESTIR_M_CLAMP
#define RESTIR_M_CLAMP pc.m_clamp
#endif
#ifndef RESTIR_M_CAP
#define RESTIR_M_CAP pc.m_cap
#endif

float m_as_f(uint value) {
    float hi = float(value >> 16u);
    float lo = float(value & 0xffffu);
    return hi * M_HI + lo;
}

float reservoir_w(uint lamp, float w_sum, uint m, float target) {
    if (is_empty(lamp, m) || target <= 0.0) {
        return 0.0;
    }
    return w_sum / (m_as_f(m) * target);
}

void cap_history(inout uint m, inout float w_sum, uint current_m) {
    if (RESTIR_M_CLAMP != 0u && current_m != 0u) {
        uint cap = RESTIR_M_CLAMP * current_m;
        if (m > cap) {
            w_sum *= m_as_f(cap) / m_as_f(m);
            m = cap;
        }
    }
    if (RESTIR_M_CAP == 0u || m <= RESTIR_M_CAP) {
        return;
    }
    w_sum *= m_as_f(RESTIR_M_CAP) / m_as_f(m);
    m = RESTIR_M_CAP;
}

// Two-reservoir balance merge. `phat_sel` is p̂ of the selected lamp at this
// receiver. Both inputs share the receiver, so source and destination match.
// An empty canonical sample returns the history record unchanged.
void merge_pair(
    uint lamp_c, float w_c, uint m_c, float target_c, float tq_c,
    uint lamp_h, float w_h, uint m_h, float target_h, float tq_h,
    float phat_of_c, float phat_of_h,
    float unit,
    out uint lamp, out float w_sum, out uint m, out float target
) {
    if (is_empty(lamp_c, m_c)) {
        if (is_empty(lamp_h, m_h)) {
            store_empty(lamp, w_sum, m, target);
        } else {
            lamp = lamp_h;
            w_sum = w_h;
            m = m_h;
            target = target_h;
        }
        return;
    }
    float mass_c = 0.0;
    if (!is_empty(lamp_c, m_c) && tq_c > 0.0) {
        mass_c = m_as_f(m_c) * reservoir_w(lamp_c, w_c, m_c, target_c) * tq_c;
    }
    float mass_h = 0.0;
    if (!is_empty(lamp_h, m_h) && tq_h > 0.0) {
        mass_h = m_as_f(m_h) * reservoir_w(lamp_h, w_h, m_h, target_h) * tq_h;
    }
    float area = mass_c + mass_h;
    if (area <= 0.0) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    bool pick_h = false;
    if (mass_h > 0.0 && unit < mass_h / area) {
        pick_h = true;
    } else if (!(mass_c > 0.0) && mass_h > 0.0) {
        pick_h = true;
    } else if (!(mass_c > 0.0)) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    uint sel = pick_h ? lamp_h : lamp_c;
    float phat_sel = pick_h ? phat_of_h : phat_of_c;
    float tq = max(phat_sel, 0.0);
    float t_src = tq;
    if (tq <= 0.0 || t_src <= 0.0) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    float denom = 0.0;
    uint count = 0u;
    if (!is_empty(lamp_c, m_c)) {
        count += m_c;
        denom += m_as_f(m_c) * tq;
    }
    if (!is_empty(lamp_h, m_h)) {
        count += m_h;
        denom += m_as_f(m_h) * tq;
    }
    if (denom <= 0.0 || count == 0u) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    float w_new = area * t_src / (tq * denom);
    lamp = sel;
    w_sum = w_new * m_as_f(count) * tq;
    m = count;
    target = tq;
}

// Two-domain generalized balance merge (Bitterli et al. 2020, Lin et al.
// 2022): the canonical (current) and history reservoirs were built for
// different points. `pc_*` is p_hat of a sample at the current point (the
// integrand), `ph_*` at the history point. Weight of sample i:
// M_i p_i(y_i) / (M_c p_c(y_i) + M_h p_h(y_i)). m_c counts an empty current
// reservoir's candidates too (lamp_c empty, M = candidates drawn).
void merge_pair_balance(
    uint lamp_c, float w_c, uint m_c, float target_c, float pc_c, float ph_c,
    uint lamp_h, float w_h, uint m_h, float target_h, float pc_h, float ph_h,
    float unit,
    out uint lamp, out float w_sum, out uint m, out float target
) {
    float mc = m_as_f(m_c);
    float mh = m_as_f(m_h);
    float wt_c = 0.0;
    if (lamp_c != LAMP_EMPTY && m_c != 0u && pc_c > 0.0) {
        float d = mc * pc_c + mh * ph_c;
        float big_w = target_c > 0.0 ? w_c / (mc * target_c) : 0.0;
        wt_c = d > 0.0 ? (mc * pc_c / d) * pc_c * big_w : 0.0;
    }
    float wt_h = 0.0;
    if (!is_empty(lamp_h, m_h) && pc_h > 0.0 && ph_h > 0.0) {
        float d = mc * pc_h + mh * ph_h;
        wt_h = d > 0.0 ? (mh * ph_h / d) * pc_h * reservoir_w(lamp_h, w_h, m_h, target_h) : 0.0;
    }
    float area = wt_c + wt_h;
    uint count = m_c + m_h;
    if (!(area > 0.0) || count == 0u) {
        store_empty(lamp, w_sum, m, target);
        return;
    }
    bool pick_h = wt_h > 0.0 && (unit < wt_h / area || !(wt_c > 0.0));
    lamp = pick_h ? lamp_h : lamp_c;
    target = pick_h ? pc_h : pc_c;
    // W = area / target; stored so that w_sum / (M * target) = W.
    w_sum = area * m_as_f(count);
    m = count;
}
