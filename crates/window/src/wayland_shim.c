#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include <xkbcommon/xkbcommon-keysyms.h>

#include "pointer-constraints-unstable-v1-client-protocol.h"
#include "relative-pointer-unstable-v1-client-protocol.h"
#include "xdg-shell-client-protocol.h"

#ifndef MFD_CLOEXEC
#define MFD_CLOEXEC 1U
#endif

struct GenosWindow {
    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_compositor *compositor;
    struct wl_surface *surface;
    struct wl_seat *seat;
    struct wl_pointer *pointer;
    struct wl_keyboard *keyboard;
    struct xdg_wm_base *wm_base;
    struct xdg_surface *xdg_surface;
    struct xdg_toplevel *toplevel;
    struct xkb_context *xkb;
    struct xkb_keymap *keymap;
    struct xkb_state *xkb_state;
    int width;
    int height;
    int configured;
    int closing;
    int focused;
    int last_x;
    int last_y;
    int have_pointer;
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
    struct zwp_pointer_constraints_v1 *constraints;
    struct zwp_relative_pointer_manager_v1 *relative_manager;
    struct zwp_locked_pointer_v1 *locked_pointer;
    struct zwp_relative_pointer_v1 *relative_pointer;
    struct wl_shm *shm;
    struct wl_shm_pool *cursor_pool;
    struct wl_buffer *cursor_buffer;
    struct wl_surface *cursor_surface;
    void *cursor_mem;
    size_t cursor_bytes;
    uint32_t pointer_serial;
    wl_fixed_t acc_dx;
    wl_fixed_t acc_dy;
    int want_capture;
    int pointer_locked;
    int clicked_while_focused;
    int fullscreen;
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
};

static void apply_cursor(struct GenosWindow *win);
static void release_pointer_lock(struct GenosWindow *win);
static int engage_pointer_lock(struct GenosWindow *win);

static void wm_ping(void *data, struct xdg_wm_base *wm, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(wm, serial);
}

static const struct xdg_wm_base_listener wm_listener = {
    .ping = wm_ping,
};

static void xdg_surface_configure(void *data, struct xdg_surface *surface, uint32_t serial) {
    struct GenosWindow *win = data;
    xdg_surface_ack_configure(surface, serial);
    win->configured = 1;
    wl_surface_commit(win->surface);
}

static const struct xdg_surface_listener xdg_surface_listener = {
    .configure = xdg_surface_configure,
};

static int has_toplevel_state(struct wl_array *states, uint32_t want) {
    uint32_t *state;
    wl_array_for_each(state, states) {
        if (*state == want) {
            return 1;
        }
    }
    return 0;
}

static void toplevel_configure(void *data, struct xdg_toplevel *toplevel, int32_t width, int32_t height, struct wl_array *states) {
    (void)toplevel;
    struct GenosWindow *win = data;
    win->fullscreen = has_toplevel_state(states, XDG_TOPLEVEL_STATE_FULLSCREEN);
    if (width > 0 && height > 0 && (width != win->width || height != win->height)) {
        win->width = width;
        win->height = height;
        win->resized = 1;
    }
}

static void toplevel_close(void *data, struct xdg_toplevel *toplevel) {
    (void)toplevel;
    ((struct GenosWindow *)data)->closing = 1;
}

static const struct xdg_toplevel_listener toplevel_listener = {
    .configure = toplevel_configure,
    .close = toplevel_close,
};

static void pointer_enter(void *data, struct wl_pointer *pointer, uint32_t serial, struct wl_surface *surface, wl_fixed_t x, wl_fixed_t y) {
    (void)pointer;
    (void)serial;
    (void)surface;
    struct GenosWindow *win = data;
    win->have_pointer = 1;
    win->pointer_serial = serial;
    win->last_x = wl_fixed_to_int(x);
    win->last_y = wl_fixed_to_int(y);
    if (win->want_capture) {
        wl_pointer_set_cursor(pointer, serial, NULL, 0, 0);
    }
}

static void pointer_leave(void *data, struct wl_pointer *pointer, uint32_t serial, struct wl_surface *surface) {
    (void)pointer;
    (void)serial;
    (void)surface;
    struct GenosWindow *win = data;
    win->have_pointer = 0;
    /* Leaving the surface must not end a capture. Take the grab again. */
    if (win->want_capture && !win->locked_pointer) {
        engage_pointer_lock(win);
    }
}

static void pointer_motion(void *data, struct wl_pointer *pointer, uint32_t time, wl_fixed_t x, wl_fixed_t y) {
    (void)pointer;
    (void)time;
    struct GenosWindow *win = data;
    int ix = wl_fixed_to_int(x);
    int iy = wl_fixed_to_int(y);
    /* A lock owns look deltas through the relative pointer. Absolute motion would double-count. */
    if (win->want_capture) {
        win->last_x = ix;
        win->last_y = iy;
        return;
    }
    if (win->have_pointer) {
        win->mouse_dx += ix - win->last_x;
        win->mouse_dy += iy - win->last_y;
    }
    win->last_x = ix;
    win->last_y = iy;
    win->have_pointer = 1;
}

static void pointer_button(void *data, struct wl_pointer *pointer, uint32_t serial, uint32_t time, uint32_t button, uint32_t state) {
    (void)pointer;
    (void)time;
    struct GenosWindow *win = data;
    win->pointer_serial = serial;
    if (button == 0x110) {
        win->mouse_left = state == WL_POINTER_BUTTON_STATE_PRESSED;
        if (win->mouse_left) {
            win->capture_click = 1;
            /* This click hit our surface. Capture even if keyboard focus is late. */
            win->clicked_while_focused = 1;
            /* The frame locks the pointer after it accepts the click. */
        }
    }
}

static void pointer_axis(void *data, struct wl_pointer *pointer, uint32_t time, uint32_t axis, wl_fixed_t value) {
    (void)data; (void)pointer; (void)time; (void)axis; (void)value;
}
static void pointer_frame(void *data, struct wl_pointer *pointer) {
    (void)data; (void)pointer;
}
static void pointer_axis_source(void *data, struct wl_pointer *pointer, uint32_t source) {
    (void)data; (void)pointer; (void)source;
}
static void pointer_axis_stop(void *data, struct wl_pointer *pointer, uint32_t time, uint32_t axis) {
    (void)data; (void)pointer; (void)time; (void)axis;
}
static void pointer_axis_discrete(void *data, struct wl_pointer *pointer, uint32_t axis, int32_t discrete) {
    (void)data; (void)pointer; (void)axis; (void)discrete;
}
static void pointer_axis_value120(void *data, struct wl_pointer *pointer, uint32_t axis, int32_t value) {
    (void)data; (void)pointer; (void)axis; (void)value;
}
static void pointer_axis_relative_direction(void *data, struct wl_pointer *pointer, uint32_t axis, uint32_t direction) {
    (void)data; (void)pointer; (void)axis; (void)direction;
}
static void pointer_warp(void *data, struct wl_pointer *pointer, wl_fixed_t x, wl_fixed_t y) {
    (void)data; (void)pointer; (void)x; (void)y;
}

static const struct wl_pointer_listener pointer_listener = {
    .enter = pointer_enter,
    .leave = pointer_leave,
    .motion = pointer_motion,
    .button = pointer_button,
    .axis = pointer_axis,
    .frame = pointer_frame,
    .axis_source = pointer_axis_source,
    .axis_stop = pointer_axis_stop,
    .axis_discrete = pointer_axis_discrete,
    .axis_value120 = pointer_axis_value120,
    .axis_relative_direction = pointer_axis_relative_direction,
    .warp = pointer_warp,
};

static void keyboard_keymap(void *data, struct wl_keyboard *keyboard, uint32_t format, int32_t fd, uint32_t size) {
    (void)keyboard;
    struct GenosWindow *win = data;
    if (format != WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1) {
        close(fd);
        return;
    }
    char *map = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (map == MAP_FAILED) {
        return;
    }
    if (win->xkb_state) {
        xkb_state_unref(win->xkb_state);
        win->xkb_state = NULL;
    }
    if (win->keymap) {
        xkb_keymap_unref(win->keymap);
        win->keymap = NULL;
    }
    win->keymap = xkb_keymap_new_from_string(win->xkb, map, XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS);
    munmap(map, size);
    if (win->keymap) {
        win->xkb_state = xkb_state_new(win->keymap);
    }
}

static void keyboard_enter(void *data, struct wl_keyboard *keyboard, uint32_t serial, struct wl_surface *surface, struct wl_array *keys) {
    (void)keyboard;
    (void)serial;
    (void)surface;
    (void)keys;
    ((struct GenosWindow *)data)->focused = 1;
}

static void keyboard_leave(void *data, struct wl_keyboard *keyboard, uint32_t serial, struct wl_surface *surface) {
    (void)keyboard;
    (void)serial;
    (void)surface;
    ((struct GenosWindow *)data)->focused = 0;
}

static void set_key(struct GenosWindow *win, xkb_keysym_t sym, int down) {
    if (sym == XKB_KEY_w || sym == XKB_KEY_W) win->key_w = down;
    if (sym == XKB_KEY_a || sym == XKB_KEY_A) win->key_a = down;
    if (sym == XKB_KEY_s || sym == XKB_KEY_S) win->key_s = down;
    if (sym == XKB_KEY_d || sym == XKB_KEY_D) win->key_d = down;
    if (sym == XKB_KEY_Escape) win->key_escape = down;
}

static void keyboard_key(void *data, struct wl_keyboard *keyboard, uint32_t serial, uint32_t time, uint32_t key, uint32_t state) {
    (void)keyboard;
    (void)serial;
    (void)time;
    struct GenosWindow *win = data;
    int down = state == WL_KEYBOARD_KEY_STATE_PRESSED;
    if (key < 256) win->keys_down[key] = (uint8_t)down;
    /* Wayland key codes are evdev codes. Keep these even if the keymap is late. */
    if (key == 1) {
        win->key_escape = down;
        if (down) release_pointer_lock(win);
    }
    if (key == 17) win->key_w = down;
    if (key == 30) win->key_a = down;
    if (key == 31) win->key_s = down;
    if (key == 32) win->key_d = down;
    if (!win->xkb_state) {
        return;
    }
    xkb_keycode_t code = key + 8;
    xkb_state_update_key(win->xkb_state, code, down ? XKB_KEY_DOWN : XKB_KEY_UP);
    xkb_keysym_t sym = xkb_state_key_get_one_sym(win->xkb_state, code);
    set_key(win, sym, down);
}

static void keyboard_modifiers(void *data, struct wl_keyboard *keyboard, uint32_t serial, uint32_t depressed, uint32_t latched, uint32_t locked, uint32_t group) {
    (void)keyboard;
    (void)serial;
    struct GenosWindow *win = data;
    if (win->xkb_state) {
        xkb_state_update_mask(win->xkb_state, depressed, latched, locked, 0, 0, group);
    }
}

static void keyboard_repeat(void *data, struct wl_keyboard *keyboard, int32_t rate, int32_t delay) {
    (void)data; (void)keyboard; (void)rate; (void)delay;
}

static const struct wl_keyboard_listener keyboard_listener = {
    .keymap = keyboard_keymap,
    .enter = keyboard_enter,
    .leave = keyboard_leave,
    .key = keyboard_key,
    .modifiers = keyboard_modifiers,
    .repeat_info = keyboard_repeat,
};

static void seat_capabilities(void *data, struct wl_seat *seat, uint32_t caps) {
    struct GenosWindow *win = data;
    if ((caps & WL_SEAT_CAPABILITY_POINTER) && !win->pointer) {
        win->pointer = wl_seat_get_pointer(seat);
        wl_pointer_add_listener(win->pointer, &pointer_listener, win);
    }
    if ((caps & WL_SEAT_CAPABILITY_KEYBOARD) && !win->keyboard) {
        win->keyboard = wl_seat_get_keyboard(seat);
        wl_keyboard_add_listener(win->keyboard, &keyboard_listener, win);
    }
}

static void seat_name(void *data, struct wl_seat *seat, const char *name) {
    (void)data;
    (void)seat;
    (void)name;
}

static const struct wl_seat_listener seat_listener = {
    .capabilities = seat_capabilities,
    .name = seat_name,
};

static void registry_global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    struct GenosWindow *win = data;
    if (strcmp(interface, wl_compositor_interface.name) == 0) {
        win->compositor = wl_registry_bind(registry, name, &wl_compositor_interface, version < 4 ? version : 4);
    } else if (strcmp(interface, xdg_wm_base_interface.name) == 0) {
        win->wm_base = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1);
        xdg_wm_base_add_listener(win->wm_base, &wm_listener, win);
    } else if (strcmp(interface, wl_seat_interface.name) == 0) {
        win->seat = wl_registry_bind(registry, name, &wl_seat_interface, version < 5 ? version : 5);
        wl_seat_add_listener(win->seat, &seat_listener, win);
    } else if (strcmp(interface, zwp_pointer_constraints_v1_interface.name) == 0) {
        win->constraints = wl_registry_bind(registry, name, &zwp_pointer_constraints_v1_interface, 1);
    } else if (strcmp(interface, zwp_relative_pointer_manager_v1_interface.name) == 0) {
        win->relative_manager = wl_registry_bind(registry, name, &zwp_relative_pointer_manager_v1_interface, 1);
    } else if (strcmp(interface, wl_shm_interface.name) == 0) {
        win->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    }
}

static void registry_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_remove,
};

static void pump_events(struct GenosWindow *win) {
    struct pollfd pfd;
    pfd.fd = wl_display_get_fd(win->display);
    pfd.events = POLLIN;
    pfd.revents = 0;
    while (wl_display_prepare_read(win->display) != 0) {
        wl_display_dispatch_pending(win->display);
    }
    wl_display_flush(win->display);
    if (poll(&pfd, 1, 8) > 0) {
        wl_display_read_events(win->display);
    } else {
        wl_display_cancel_read(win->display);
    }
    wl_display_dispatch_pending(win->display);
}

static void on_locked(void *data, struct zwp_locked_pointer_v1 *lock) {
    (void)lock;
    struct GenosWindow *win = data;
    win->pointer_locked = 1;
    apply_cursor(win);
}

static void on_unlocked(void *data, struct zwp_locked_pointer_v1 *lock) {
    (void)lock;
    ((struct GenosWindow *)data)->pointer_locked = 0;
}

static const struct zwp_locked_pointer_v1_listener locked_listener = {
    .locked = on_locked,
    .unlocked = on_unlocked,
};

static void on_relative_motion(
    void *data,
    struct zwp_relative_pointer_v1 *pointer,
    uint32_t utime_hi,
    uint32_t utime_lo,
    wl_fixed_t dx,
    wl_fixed_t dy,
    wl_fixed_t dx_unaccel,
    wl_fixed_t dy_unaccel
) {
    (void)pointer;
    (void)utime_hi;
    (void)utime_lo;
    (void)dx_unaccel;
    (void)dy_unaccel;
    struct GenosWindow *win = data;
    win->acc_dx += dx;
    win->acc_dy += dy;
    int ix = wl_fixed_to_int(win->acc_dx);
    int iy = wl_fixed_to_int(win->acc_dy);
    win->acc_dx -= wl_fixed_from_int(ix);
    win->acc_dy -= wl_fixed_from_int(iy);
    win->mouse_dx += ix;
    win->mouse_dy += iy;
}

static const struct zwp_relative_pointer_v1_listener relative_listener = {
    .relative_motion = on_relative_motion,
};

static void make_cursor(struct GenosWindow *win) {
    const int size = 16;
    const int stride = size * 4;
    const int bytes = stride * size;
    int fd = (int)syscall(SYS_memfd_create, "genos-cursor", MFD_CLOEXEC);
    if (fd < 0) {
        return;
    }
    if (ftruncate(fd, bytes) != 0) {
        close(fd);
        return;
    }
    void *mem = mmap(NULL, (size_t)bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (mem == MAP_FAILED) {
        close(fd);
        return;
    }
    uint8_t *pixels = mem;
    memset(pixels, 0, (size_t)bytes);
    for (int y = 0; y < size; y++) {
        for (int x = 0; x < size; x++) {
            int edge = (x == 0 || y == 0 || x == y) && x < 12 && y < 12;
            if (!edge) {
                continue;
            }
            uint8_t *px = pixels + (y * stride) + x * 4;
            px[0] = 255;
            px[1] = 255;
            px[2] = 255;
            px[3] = 255;
        }
    }
    win->cursor_pool = wl_shm_create_pool(win->shm, fd, bytes);
    close(fd);
    if (!win->cursor_pool) {
        munmap(mem, (size_t)bytes);
        return;
    }
    win->cursor_buffer = wl_shm_pool_create_buffer(win->cursor_pool, 0, size, size, stride, WL_SHM_FORMAT_ARGB8888);
    win->cursor_surface = wl_compositor_create_surface(win->compositor);
    if (!win->cursor_buffer || !win->cursor_surface) {
        munmap(mem, (size_t)bytes);
        return;
    }
    wl_surface_attach(win->cursor_surface, win->cursor_buffer, 0, 0);
    wl_surface_commit(win->cursor_surface);
    win->cursor_mem = mem;
    win->cursor_bytes = (size_t)bytes;
}

static void apply_cursor(struct GenosWindow *win) {
    if (!win->pointer || win->pointer_serial == 0) {
        return;
    }
    if (win->want_capture) {
        wl_pointer_set_cursor(win->pointer, win->pointer_serial, NULL, 0, 0);
    } else if (win->cursor_surface) {
        wl_pointer_set_cursor(win->pointer, win->pointer_serial, win->cursor_surface, 0, 0);
    }
}

static int engage_pointer_lock(struct GenosWindow *win) {
    if (win->locked_pointer) {
        return 0;
    }
    if (!win->constraints || !win->relative_manager || !win->pointer || !win->surface) {
        fprintf(stderr, "genos-window: pointer lock is unavailable\n");
        return -1;
    }
    win->want_capture = 1;
    win->locked_pointer = zwp_pointer_constraints_v1_lock_pointer(
        win->constraints,
        win->surface,
        win->pointer,
        NULL,
        ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_PERSISTENT
    );
    if (!win->locked_pointer) {
        win->want_capture = 0;
        fprintf(stderr, "genos-window: lock_pointer failed\n");
        return -1;
    }
    zwp_locked_pointer_v1_add_listener(win->locked_pointer, &locked_listener, win);
    win->relative_pointer = zwp_relative_pointer_manager_v1_get_relative_pointer(win->relative_manager, win->pointer);
    if (!win->relative_pointer) {
        release_pointer_lock(win);
        fprintf(stderr, "genos-window: relative pointer failed\n");
        return -1;
    }
    zwp_relative_pointer_v1_add_listener(win->relative_pointer, &relative_listener, win);
    apply_cursor(win);
    wl_display_flush(win->display);
    return 0;
}

static void release_pointer_lock(struct GenosWindow *win) {
    win->want_capture = 0;
    if (win->relative_pointer) {
        zwp_relative_pointer_v1_destroy(win->relative_pointer);
        win->relative_pointer = NULL;
    }
    if (win->locked_pointer) {
        zwp_locked_pointer_v1_set_cursor_position_hint(
            win->locked_pointer,
            wl_fixed_from_int(win->last_x),
            wl_fixed_from_int(win->last_y)
        );
        wl_surface_commit(win->surface);
        zwp_locked_pointer_v1_destroy(win->locked_pointer);
        win->locked_pointer = NULL;
    }
    win->pointer_locked = 0;
    apply_cursor(win);
}

struct GenosWindow *genos_window_open(int width, int height, const char *app_id, const char *title) {
    struct GenosWindow *win = calloc(1, sizeof(*win));
    if (!win) {
        return NULL;
    }
    win->width = width > 0 ? width : 1280;
    win->height = height > 0 ? height : 720;
    win->display = wl_display_connect(NULL);
    if (!win->display) {
        free(win);
        return NULL;
    }
    win->xkb = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    win->registry = wl_display_get_registry(win->display);
    wl_registry_add_listener(win->registry, &registry_listener, win);
    wl_display_roundtrip(win->display);
    wl_display_roundtrip(win->display);
    if (!win->compositor || !win->wm_base || !win->constraints || !win->relative_manager) {
        fprintf(
            stderr,
            "genos-window: missing %s\n",
            !win->compositor ? "wl_compositor" :
            !win->wm_base ? "xdg_wm_base" :
            !win->constraints ? "zwp_pointer_constraints_v1" :
            "zwp_relative_pointer_manager_v1"
        );
        if (win->xkb) xkb_context_unref(win->xkb);
        wl_display_disconnect(win->display);
        free(win);
        return NULL;
    }
    win->surface = wl_compositor_create_surface(win->compositor);
    if (win->shm) {
        make_cursor(win);
    } else {
        fprintf(stderr, "genos-window: wl_shm is missing, pointer restore has no cursor\n");
    }
    win->xdg_surface = xdg_wm_base_get_xdg_surface(win->wm_base, win->surface);
    xdg_surface_add_listener(win->xdg_surface, &xdg_surface_listener, win);
    win->toplevel = xdg_surface_get_toplevel(win->xdg_surface);
    xdg_toplevel_add_listener(win->toplevel, &toplevel_listener, win);
    if (!app_id || app_id[0] == '\0') {
        app_id = "genos";
    }
    if (!title || title[0] == '\0') {
        title = "Genos";
    }
    xdg_toplevel_set_app_id(win->toplevel, app_id);
    xdg_toplevel_set_title(win->toplevel, title);
    wl_surface_commit(win->surface);
    while (!win->configured) {
        if (wl_display_dispatch(win->display) < 0) {
            break;
        }
    }
    return win;
}

void genos_window_pump(struct GenosWindow *win, struct GenosPump *out) {
    if (!win || !out) {
        return;
    }
    pump_events(win);
    out->width = win->width;
    out->height = win->height;
    out->closing = win->closing;
    out->focused = win->focused;
    out->mouse_dx = win->mouse_dx;
    out->mouse_dy = win->mouse_dy;
    out->capture_click = win->capture_click;
    out->key_w = win->key_w;
    out->key_a = win->key_a;
    out->key_s = win->key_s;
    out->key_d = win->key_d;
    out->key_escape = win->key_escape;
    out->resized = win->resized;
    out->mouse_left = win->mouse_left;
    memcpy(out->keys_down, win->keys_down, sizeof(out->keys_down));
    out->pointer_locked = win->pointer_locked;
    out->clicked_while_focused = win->clicked_while_focused;
    out->fullscreen = win->fullscreen;
    out->pointer_x = win->last_x;
    out->pointer_y = win->last_y;
    out->display = win->display;
    out->surface = win->surface;
    win->mouse_dx = 0;
    win->mouse_dy = 0;
    win->capture_click = 0;
    win->clicked_while_focused = 0;
    win->resized = 0;
}

int genos_window_pump_size(void) {
    return (int)sizeof(struct GenosPump);
}

int genos_window_set_fullscreen(struct GenosWindow *win, int fullscreen) {
    if (!win || !win->toplevel) {
        return -1;
    }
    if (fullscreen) {
        xdg_toplevel_set_fullscreen(win->toplevel, NULL);
    } else {
        xdg_toplevel_unset_fullscreen(win->toplevel);
    }
    wl_surface_commit(win->surface);
    wl_display_flush(win->display);
    return 0;
}

int genos_window_set_pointer_capture(struct GenosWindow *win, int capture) {
    if (!win) {
        return -1;
    }
    if (!capture) {
        release_pointer_lock(win);
        wl_display_flush(win->display);
        return 0;
    }
    if (engage_pointer_lock(win) != 0) {
        return -1;
    }
    wl_display_roundtrip(win->display);
    /* The proxy existing is not a grab. The compositor must confirm it. */
    if (!win->pointer_locked) {
        release_pointer_lock(win);
        wl_display_flush(win->display);
        fprintf(stderr, "genos-window: compositor did not lock the pointer\n");
        return -1;
    }
    apply_cursor(win);
    wl_display_flush(win->display);
    return 0;
}

void genos_window_destroy(struct GenosWindow *win) {
    if (!win) {
        return;
    }
    release_pointer_lock(win);
    if (win->cursor_surface) wl_surface_destroy(win->cursor_surface);
    if (win->cursor_buffer) wl_buffer_destroy(win->cursor_buffer);
    if (win->cursor_pool) wl_shm_pool_destroy(win->cursor_pool);
    if (win->cursor_mem) munmap(win->cursor_mem, win->cursor_bytes);
    if (win->constraints) zwp_pointer_constraints_v1_destroy(win->constraints);
    if (win->relative_manager) zwp_relative_pointer_manager_v1_destroy(win->relative_manager);
    if (win->shm) wl_shm_destroy(win->shm);
    if (win->xkb_state) xkb_state_unref(win->xkb_state);
    if (win->keymap) xkb_keymap_unref(win->keymap);
    if (win->xkb) xkb_context_unref(win->xkb);
    if (win->toplevel) xdg_toplevel_destroy(win->toplevel);
    if (win->xdg_surface) xdg_surface_destroy(win->xdg_surface);
    if (win->surface) wl_surface_destroy(win->surface);
    if (win->pointer) wl_pointer_destroy(win->pointer);
    if (win->keyboard) wl_keyboard_destroy(win->keyboard);
    if (win->seat) wl_seat_destroy(win->seat);
    if (win->wm_base) xdg_wm_base_destroy(win->wm_base);
    if (win->compositor) wl_compositor_destroy(win->compositor);
    if (win->registry) wl_registry_destroy(win->registry);
    if (win->display) wl_display_disconnect(win->display);
    free(win);
}
