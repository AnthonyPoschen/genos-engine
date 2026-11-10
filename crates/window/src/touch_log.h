#ifndef GENOS_TOUCH_LOG_H
#define GENOS_TOUCH_LOG_H

#include "genos_pump.h"

/* One slot per finger for this pump, plus the fingers still down. */
struct GenosTouchLog {
    struct GenosTouch events[GENOS_TOUCH_CAP];
    int event_count;
    struct GenosTouch active[GENOS_TOUCH_CAP];
    int active_count;
};

int genos_touch_log_size(void);
void genos_touch_log_push(struct GenosTouchLog *log, uint64_t id, float x, float y, int phase);
void genos_touch_log_cancel(struct GenosTouchLog *log);
int genos_touch_log_copy(struct GenosTouchLog *log, struct GenosTouch *out);

#endif
