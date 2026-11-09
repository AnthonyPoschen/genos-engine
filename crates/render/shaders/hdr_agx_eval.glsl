// AgX as `agx_rec709`: Rec.709 to Rec.2020, lower guard, inset, the log sigmoid,
// a 40% hue mix, outset, back to Rec.709, then the power-2.4 code.
// Rows are stored as columns so a dot is one row of the CPU matrix.

const float AGX_HUE = 0.40;
const float AGX_MID = 0.18;
const float AGX_LOG_MIN_C = -10.0;
const float AGX_LOG_MAX_C = 6.5;
const float AGX_SLOPE = 2.4;
const float AGX_POWER = 1.5;

vec3 agx_mul(vec3 r0, vec3 r1, vec3 r2, vec3 v) {
    return vec3(dot(r0, v), dot(r1, v), dot(r2, v));
}

float agx_scale_factor(float limit_x, float limit_y, float transition_x, float transition_y) {
    float span_x = limit_x - transition_x;
    float span_y = limit_y - transition_y;
    float term_a = pow(AGX_SLOPE * span_x, -AGX_POWER);
    float term_b = pow(AGX_SLOPE * span_x / span_y, AGX_POWER) - 1.0;
    return pow(term_a * term_b, -1.0 / AGX_POWER);
}

float agx_exp_curve(float x, float scale, float transition_x, float transition_y) {
    float inner = AGX_SLOPE * (x - transition_x) / scale;
    float signed_p = inner < 0.0 ? -pow(-inner, AGX_POWER) : pow(inner, AGX_POWER);
    return scale * (inner / pow(1.0 + signed_p, 1.0 / AGX_POWER)) + transition_y;
}

float agx_sigmoid(float x) {
    float x_pivot = (-AGX_LOG_MIN_C) / (AGX_LOG_MAX_C - AGX_LOG_MIN_C);
    float y_pivot = pow(AGX_MID, 1.0 / AGX_SLOPE);
    float scale_toe = -agx_scale_factor(1.0, 1.0, 1.0 - x_pivot, 1.0 - y_pivot);
    float scale_shoulder = agx_scale_factor(1.0, 1.0, x_pivot, y_pivot);
    if (x < x_pivot) {
        return agx_exp_curve(x, scale_toe, x_pivot, y_pivot);
    }
    return agx_exp_curve(x, scale_shoulder, x_pivot, y_pivot);
}

float agx_curve(float x) {
    if (!(x > 0.0)) {
        return 0.0;
    }
    float logged = (log2(x / AGX_MID) - AGX_LOG_MIN_C) / (AGX_LOG_MAX_C - AGX_LOG_MIN_C);
    float curved = agx_sigmoid(logged);
    if (!(curved > 0.0)) {
        return 0.0;
    }
    return pow(curved, 2.4);
}

vec3 agx_compensate(vec3 rgb) {
    vec3 y_w = vec3(0.2589235355689848, 0.6104985346066525, 0.13057792982436284);
    float y = dot(rgb, y_w);
    float peak = max(rgb.r, max(rgb.g, rgb.b));
    vec3 inverse = vec3(peak) - rgb;
    float max_inverse = max(inverse.r, max(inverse.g, inverse.b));
    float y_compensate = max_inverse - dot(inverse, y_w) + y;
    float min_rgb = min(rgb.r, min(rgb.g, rgb.b));
    float offset = max(-min_rgb, 0.0);
    vec3 shifted = rgb + vec3(offset);
    float peak_shifted = max(shifted.r, max(shifted.g, shifted.b));
    vec3 inverse_shifted = vec3(peak_shifted) - shifted;
    float max_inverse_shifted = max(inverse_shifted.r, max(inverse_shifted.g, inverse_shifted.b));
    float y_new = max_inverse_shifted - dot(inverse_shifted, y_w) + dot(shifted, y_w);
    float ratio = y_new > y_compensate ? y_compensate / y_new : 1.0;
    return shifted * ratio;
}

vec3 agx_rgb_to_hsv(vec3 rgb) {
    float peak = max(rgb.r, max(rgb.g, rgb.b));
    float floor_v = min(rgb.r, min(rgb.g, rgb.b));
    float delta = peak - floor_v;
    float saturation = peak == 0.0 ? 0.0 : delta / peak;
    float hue = 0.0;
    if (delta != 0.0) {
        if (rgb.r >= rgb.g && rgb.r >= rgb.b) {
            float sector = (rgb.g - rgb.b) / delta;
            if (sector < 0.0) {
                sector += 6.0;
            }
            hue = sector / 6.0;
        } else if (rgb.g >= rgb.b) {
            hue = ((rgb.b - rgb.r) / delta + 2.0) / 6.0;
        } else {
            hue = ((rgb.r - rgb.g) / delta + 4.0) / 6.0;
        }
    }
    return vec3(hue, saturation, peak);
}

vec3 agx_hsv_to_rgb(vec3 hsv) {
    float hue = mod(hsv.x, 1.0);
    if (hue < 0.0) {
        hue += 1.0;
    }
    float saturation = hsv.y;
    float value = hsv.z;
    if (saturation == 0.0) {
        return vec3(value);
    }
    float sector_f = hue * 6.0;
    float sector = floor(sector_f);
    float fraction = sector_f - sector;
    float p = value * (1.0 - saturation);
    float q = value * (1.0 - saturation * fraction);
    float t = value * (1.0 - saturation * (1.0 - fraction));
    if (sector < 1.0) {
        return vec3(value, t, p);
    }
    if (sector < 2.0) {
        return vec3(q, value, p);
    }
    if (sector < 3.0) {
        return vec3(p, value, t);
    }
    if (sector < 4.0) {
        return vec3(p, q, value);
    }
    if (sector < 5.0) {
        return vec3(t, p, value);
    }
    return vec3(value, p, q);
}

float agx_lerp_hue(float start, float end, float t) {
    float delta = end - start;
    float wrapped = end;
    if (delta > 0.5) {
        wrapped = end - 1.0;
    } else if (delta < -0.5) {
        wrapped = end + 1.0;
    }
    float hue = start + t * (wrapped - start);
    hue = mod(hue, 1.0);
    if (hue < 0.0) {
        hue += 1.0;
    }
    return hue;
}

float agx_encode(float x) {
    float encoded = x <= 0.0 ? 0.0 : pow(x, 1.0 / 2.4);
    return clamp(encoded, 0.0, 1.0);
}

vec3 agx_base(vec3 color) {
    vec3 guarded = agx_compensate(color);
    vec3 inset = agx_mul(
        vec3(0.8566271562887792, 0.0951212454025349, 0.0482515983086858),
        vec3(0.13731897228355167, 0.7612419870090806, 0.10143904070736748),
        vec3(0.11189820804517953, 0.07679941456251757, 0.811302377392303),
        guarded
    );
    vec3 pre = agx_rgb_to_hsv(inset);
    vec3 linear = vec3(agx_curve(inset.r), agx_curve(inset.g), agx_curve(inset.b));
    vec3 post = agx_rgb_to_hsv(linear);
    post.x = agx_lerp_hue(pre.x, post.x, AGX_HUE);
    return agx_mul(
        vec3(1.12710056963012, -0.11060663857826065, -0.01649393105185902),
        vec3(-0.14132975442135337, 1.1578236854732127, -0.01649393105185903),
        vec3(-0.14132975442135326, -0.11060663857826059, 1.251936392999614),
        agx_hsv_to_rgb(post)
    );
}

vec3 agx_eval(vec3 scene) {
    vec3 rec2020 = agx_mul(
        vec3(1.7166634277958805, -0.3556733197301399, -0.2533680878902478),
        vec3(-0.6666738361988869, 1.616455739824698, 0.0157682970961337),
        vec3(0.0176424817849772, -0.0427769763827532, 0.9422432810184308),
        agx_mul(
            vec3(0.4123865632529918, 0.3575914909206254, 0.1804504912035637),
            vec3(0.2126368216773238, 0.7151829818412507, 0.0721801964814255),
            vec3(0.019330620152484, 0.1191971636402085, 0.9503725870054355),
            scene
        )
    );
    vec3 formed = agx_base(rec2020);
    vec3 rec709 = agx_mul(
        vec3(3.241003232976359, -1.5373989694887855, -0.4986158819963629),
        vec3(-0.9692242522025164, 1.875929983695176, 0.04155422634008472),
        vec3(0.05563941985197547, -0.20401120612390994, 1.0571489771875333),
        agx_mul(
            vec3(0.636953506785074, 0.1446191846692331, 0.1688558539228734),
            vec3(0.262698338956556, 0.6780087657728165, 0.0592928952706273),
            vec3(0.0, 0.028073135847557, 1.0608272349505707),
            formed
        )
    );
    vec3 guarded = agx_compensate(rec709);
    return vec3(agx_encode(guarded.r), agx_encode(guarded.g), agx_encode(guarded.b));
}
