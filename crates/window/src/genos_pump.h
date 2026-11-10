#ifndef GENOS_PUMP_H
#define GENOS_PUMP_H

#include <stdint.h>

/* The Rust `RawPump` record matches this layout. `genos_window_pump_size` is the check. */
enum {
    GENOS_BACKEND_WAYLAND = 1,
    GENOS_BACKEND_X11 = 2,
    GENOS_BACKEND_WIN32 = 3,
    GENOS_BACKEND_METAL = 4
};

struct GenosPump {
    int width;
    int height;
    int closing;
    int focused;
    int mouse_dx;
    int mouse_dy;
    int capture_click;
    int key_w;
    int key_a;
    int key_s;
    int key_d;
    int key_escape;
    int resized;
    int mouse_left;
    uint8_t keys_down[256];
    int pointer_locked;
    int clicked_while_focused;
    int fullscreen;
    int pointer_x;
    int pointer_y;
    void *display;
    void *surface;
    double wheel_y;
};

/* 1 Wayland, 2 X11, 3 Win32, 4 Metal. The renderer reads this when it creates the surface. */
int genos_window_backend(void);

/* Touch samples beside the pump. The pump record stays the Wayland size. */
enum {
    GENOS_TOUCH_BEGAN = 0,
    GENOS_TOUCH_MOVED = 1,
    GENOS_TOUCH_ENDED = 2,
    GENOS_TOUCH_CANCELLED = 3,
    GENOS_TOUCH_CAP = 8
};

struct GenosTouch {
    uint64_t id;
    float x;
    float y;
    int phase;
};

struct GenosExtra {
    int mouse_right;
    int mouse_middle;
    int touch_count;
    struct GenosTouch touch[GENOS_TOUCH_CAP];
};

#endif
