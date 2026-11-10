#include "touch_log.h"

#include <string.h>

int genos_touch_log_size(void) {
    return (int)sizeof(struct GenosTouchLog);
}

static int find_id(const struct GenosTouch *samples, int count, uint64_t id) {
    int i;
    for (i = 0; i < count; i++) {
        if (samples[i].id == id) {
            return i;
        }
    }
    return -1;
}

static void write_sample(struct GenosTouch *sample, uint64_t id, float x, float y, int phase) {
    sample->id = id;
    sample->x = x;
    sample->y = y;
    sample->phase = phase;
}

/* A move can be rebuilt from the fingers still down. A lift cannot. */
static int evict_moved(const struct GenosTouchLog *log) {
    int i;
    for (i = 0; i < log->event_count; i++) {
        if (log->events[i].phase == GENOS_TOUCH_MOVED) {
            return i;
        }
    }
    return -1;
}

/* Keep a lift. Drop a move when the eight slots are full. A down is replaced
 * only when nothing else can make room, so the lift still leaves this pump. */
static void store_lift(struct GenosTouchLog *log, int began, uint64_t id, float x, float y, int phase) {
    int victim;
    if (log->event_count < GENOS_TOUCH_CAP) {
        write_sample(&log->events[log->event_count], id, x, y, phase);
        log->event_count += 1;
        return;
    }
    victim = evict_moved(log);
    if (victim >= 0) {
        write_sample(&log->events[victim], id, x, y, phase);
        return;
    }
    if (began >= 0) {
        write_sample(&log->events[began], id, x, y, phase);
    }
}

void genos_touch_log_push(struct GenosTouchLog *log, uint64_t id, float x, float y, int phase) {
    int slot;
    int active;
    if (!log) {
        return;
    }
    slot = find_id(log->events, log->event_count, id);
    /* A move leaves a down where it began. The position is updated below. */
    if (!(slot >= 0 && log->events[slot].phase == GENOS_TOUCH_BEGAN && phase == GENOS_TOUCH_MOVED)) {
        if (phase == GENOS_TOUCH_ENDED || phase == GENOS_TOUCH_CANCELLED) {
            int began = -1;
            if (slot >= 0 && log->events[slot].phase == GENOS_TOUCH_BEGAN) {
                began = slot;
            }
            store_lift(log, began, id, x, y, phase);
        } else if (slot >= 0) {
            write_sample(&log->events[slot], id, x, y, phase);
        } else if (log->event_count < GENOS_TOUCH_CAP) {
            write_sample(&log->events[log->event_count], id, x, y, phase);
            log->event_count += 1;
        }
    }
    if (phase == GENOS_TOUCH_ENDED || phase == GENOS_TOUCH_CANCELLED) {
        active = find_id(log->active, log->active_count, id);
        if (active >= 0) {
            log->active[active] = log->active[log->active_count - 1];
            log->active_count -= 1;
        }
        return;
    }
    active = find_id(log->active, log->active_count, id);
    if (active >= 0) {
        log->active[active].x = x;
        log->active[active].y = y;
        return;
    }
    if (log->active_count >= GENOS_TOUCH_CAP) {
        return;
    }
    write_sample(&log->active[log->active_count], id, x, y, GENOS_TOUCH_MOVED);
    log->active_count += 1;
}

void genos_touch_log_cancel(struct GenosTouchLog *log) {
    while (log && log->active_count > 0) {
        struct GenosTouch sample = log->active[0];
        genos_touch_log_push(log, sample.id, sample.x, sample.y, GENOS_TOUCH_CANCELLED);
    }
}

int genos_touch_log_copy(struct GenosTouchLog *log, struct GenosTouch *out) {
    int count;
    int i;
    if (!log || !out) {
        return 0;
    }
    if (log->event_count > 0) {
        count = log->event_count;
        if (count > GENOS_TOUCH_CAP) {
            count = GENOS_TOUCH_CAP;
        }
        memcpy(out, log->events, (size_t)count * sizeof(out[0]));
        log->event_count = 0;
        for (i = 0; i < log->active_count && count < GENOS_TOUCH_CAP; i++) {
            if (find_id(out, count, log->active[i].id) >= 0) {
                continue;
            }
            out[count] = log->active[i];
            out[count].phase = GENOS_TOUCH_MOVED;
            count += 1;
        }
        return count;
    }
    count = log->active_count;
    if (count > GENOS_TOUCH_CAP) {
        count = GENOS_TOUCH_CAP;
    }
    for (i = 0; i < count; i++) {
        out[i] = log->active[i];
        out[i].phase = GENOS_TOUCH_MOVED;
    }
    return count;
}
