/* X11 fallback. Wayland is tried first. This file opens when that connect fails.
 * `display` is `Display *`. `surface` is the window id.
 * Keys are X keycodes minus 8, the evdev XKB mapping, so left and right
 * modifiers stay apart. A grabbed pointer reads `XI_RawMotion`.
 * This file does not read `/dev/input/jsN`. The input crate already does. */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/extensions/XInput2.h>
#include <X11/extensions/Xfixes.h>

#include "genos_pump.h"
#include "linux_host.h"

extern int genos_evdev_from_x_keycode(unsigned keycode);

struct GenosX11 {
    Display *display;
    Window window;
    Window root;
    int xi_opcode;
    int xi_on;
    int xfixes_on;
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
    int mouse_right;
    int mouse_middle;
    uint8_t keys_down[256];
    int pointer_locked;
    int want_capture;
    int clicked_while_focused;
    int fullscreen;
    int last_x;
    int last_y;
    int have_pointer;
    int cursor_hidden;
    int minimized;
    double wheel_y;
    Atom wm_protocols;
    Atom wm_delete;
    Atom wm_state;
    Atom net_wm_state;
    Atom net_wm_fullscreen;
    struct GenosTouch events[GENOS_TOUCH_CAP];
    int event_count;
    struct GenosTouch active[GENOS_TOUCH_CAP];
    int active_count;
};

static void suspend_grab(struct GenosX11 *win);
static void clear_keys(struct GenosX11 *win);
static void cancel_touch(struct GenosX11 *win);

static int round_delta(double value) {
    if (value >= 0.0) {
        return (int)(value + 0.5);
    }
    return (int)(value - 0.5);
}

static void clear_keys(struct GenosX11 *win) {
    memset(win->keys_down, 0, sizeof(win->keys_down));
    win->key_w = 0;
    win->key_a = 0;
    win->key_s = 0;
    win->key_d = 0;
    win->key_escape = 0;
    win->mouse_left = 0;
    win->mouse_right = 0;
    win->mouse_middle = 0;
}

static void set_evdev(struct GenosX11 *win, int evdev, int down) {
    if (evdev < 0 || evdev > 255) {
        return;
    }
    win->keys_down[evdev] = (uint8_t)(down ? 1 : 0);
    if (evdev == 17) {
        win->key_w = down;
    }
    if (evdev == 30) {
        win->key_a = down;
    }
    if (evdev == 31) {
        win->key_s = down;
    }
    if (evdev == 32) {
        win->key_d = down;
    }
    if (evdev == 1) {
        win->key_escape = down;
    }
}

static void push_touch(struct GenosX11 *win, uint64_t id, float x, float y, int phase) {
    int i;
    if (win->event_count < GENOS_TOUCH_CAP) {
        struct GenosTouch *sample = &win->events[win->event_count];
        sample->id = id;
        sample->x = x;
        sample->y = y;
        sample->phase = phase;
        win->event_count += 1;
    }
    if (phase == GENOS_TOUCH_ENDED || phase == GENOS_TOUCH_CANCELLED) {
        for (i = 0; i < win->active_count; i++) {
            if (win->active[i].id == id) {
                win->active[i] = win->active[win->active_count - 1];
                win->active_count -= 1;
                return;
            }
        }
        return;
    }
    for (i = 0; i < win->active_count; i++) {
        if (win->active[i].id == id) {
            win->active[i].x = x;
            win->active[i].y = y;
            return;
        }
    }
    if (win->active_count >= GENOS_TOUCH_CAP) {
        return;
    }
    win->active[win->active_count].id = id;
    win->active[win->active_count].x = x;
    win->active[win->active_count].y = y;
    win->active[win->active_count].phase = GENOS_TOUCH_MOVED;
    win->active_count += 1;
}

static void cancel_touch(struct GenosX11 *win) {
    while (win->active_count > 0) {
        struct GenosTouch sample = win->active[0];
        push_touch(win, sample.id, sample.x, sample.y, GENOS_TOUCH_CANCELLED);
    }
}

static void note_motion(struct GenosX11 *win, int x, int y) {
    if (win->have_pointer && !(win->pointer_locked && win->xi_on)) {
        win->mouse_dx += x - win->last_x;
        win->mouse_dy += y - win->last_y;
    }
    win->last_x = x;
    win->last_y = y;
    win->have_pointer = 1;
}

static double raw_axis(XIRawEvent *raw, int axis) {
    int bit;
    int seen = 0;
    if (!raw || !raw->valuators.mask || !raw->valuators.values) {
        return 0.0;
    }
    for (bit = 0; bit < raw->valuators.mask_len * 8; bit++) {
        if (!XIMaskIsSet(raw->valuators.mask, bit)) {
            continue;
        }
        if (bit == axis) {
            return raw->valuators.values[seen];
        }
        seen += 1;
    }
    return 0.0;
}

static void note_raw(struct GenosX11 *win, XIRawEvent *raw) {
    if (!win->pointer_locked || !win->xi_on) {
        return;
    }
    win->mouse_dx += round_delta(raw_axis(raw, 0));
    win->mouse_dy += round_delta(raw_axis(raw, 1));
}

static void note_button(struct GenosX11 *win, unsigned button, int down) {
    int *flag;
    if (button == 4 && down) {
        win->wheel_y += 1.0;
        return;
    }
    if (button == 5 && down) {
        win->wheel_y -= 1.0;
        return;
    }
    if (button == 1) {
        flag = &win->mouse_left;
    } else if (button == 2) {
        flag = &win->mouse_middle;
    } else if (button == 3) {
        flag = &win->mouse_right;
    } else {
        return;
    }
    *flag = down ? 1 : 0;
    if (down && button == 1) {
        win->capture_click = 1;
        /* The click hit this window. Capture even if keyboard focus is late. */
        win->clicked_while_focused = 1;
    }
}

static int repeat_release(struct GenosX11 *win, const XEvent *ev) {
    XEvent next;
    if (ev->type != KeyRelease) {
        return 0;
    }
    if (!XEventsQueued(win->display, QueuedAfterReading)) {
        return 0;
    }
    XPeekEvent(win->display, &next);
    if (next.type != KeyPress || next.xkey.keycode != ev->xkey.keycode || next.xkey.time != ev->xkey.time) {
        return 0;
    }
    XNextEvent(win->display, &next);
    return 1;
}

static void note_key(struct GenosX11 *win, const XEvent *ev) {
    int evdev;
    if (repeat_release(win, ev)) {
        return;
    }
    evdev = genos_evdev_from_x_keycode(ev->xkey.keycode);
    set_evdev(win, evdev, ev->type == KeyPress);
}

static void note_wm_state(struct GenosX11 *win) {
    Atom type = 0;
    int format = 0;
    unsigned long count = 0;
    unsigned long after = 0;
    unsigned char *data = NULL;
    long state;
    int was;
    if (XGetWindowProperty(
            win->display,
            win->window,
            win->wm_state,
            0,
            2,
            False,
            AnyPropertyType,
            &type,
            &format,
            &count,
            &after,
            &data
        ) != Success || !data || count < 1) {
        if (data) {
            XFree(data);
        }
        return;
    }
    state = ((long *)data)[0];
    XFree(data);
    was = win->minimized;
    win->minimized = state == IconicState;
    if (win->minimized) {
        suspend_grab(win);
        return;
    }
    if (was) {
        win->resized = 1;
        if (win->want_capture && win->focused) {
            genos_x11_set_pointer_capture(win, 1);
        }
    }
}

static void note_net_state(struct GenosX11 *win) {
    Atom type = 0;
    int format = 0;
    unsigned long count = 0;
    unsigned long after = 0;
    unsigned char *data = NULL;
    unsigned long i;
    int on = 0;
    if (XGetWindowProperty(
            win->display,
            win->window,
            win->net_wm_state,
            0,
            64,
            False,
            XA_ATOM,
            &type,
            &format,
            &count,
            &after,
            &data
        ) != Success || !data) {
        if (data) {
            XFree(data);
        }
        return;
    }
    if (format == 32) {
        long *atoms = (long *)data;
        for (i = 0; i < count; i++) {
            if (atoms[i] == (long)win->net_wm_fullscreen) {
                on = 1;
            }
        }
    }
    win->fullscreen = on;
    XFree(data);
}

static void note_xi(struct GenosX11 *win, int evtype, void *data) {
    if (evtype == XI_RawMotion) {
        note_raw(win, data);
        return;
    }
    if (evtype == XI_TouchBegin || evtype == XI_TouchUpdate || evtype == XI_TouchEnd) {
        XIDeviceEvent *touch = data;
        int phase = GENOS_TOUCH_MOVED;
        if (touch->event != win->window) {
            return;
        }
        if (evtype == XI_TouchBegin) {
            phase = GENOS_TOUCH_BEGAN;
        } else if (evtype == XI_TouchEnd) {
            phase = GENOS_TOUCH_ENDED;
        }
        push_touch(win, (uint64_t)touch->detail, (float)touch->event_x, (float)touch->event_y, phase);
    }
}

static void handle_core(struct GenosX11 *win, XEvent *ev) {
    if (ev->type == KeyPress || ev->type == KeyRelease) {
        note_key(win, ev);
        return;
    }
    if (ev->type == ButtonPress || ev->type == ButtonRelease) {
        note_button(win, ev->xbutton.button, ev->type == ButtonPress);
        return;
    }
    if (ev->type == MotionNotify) {
        note_motion(win, ev->xmotion.x, ev->xmotion.y);
        return;
    }
    if (ev->type == FocusIn || ev->type == FocusOut) {
        if (ev->xfocus.mode == NotifyGrab || ev->xfocus.mode == NotifyUngrab) {
            return;
        }
        if (ev->type == FocusIn) {
            win->focused = 1;
            if (win->want_capture && !win->pointer_locked && !win->minimized) {
                genos_x11_set_pointer_capture(win, 1);
            }
            return;
        }
        win->focused = 0;
        clear_keys(win);
        cancel_touch(win);
        suspend_grab(win);
        return;
    }
    if (ev->type == ConfigureNotify) {
        if (ev->xconfigure.width != win->width || ev->xconfigure.height != win->height) {
            win->width = ev->xconfigure.width;
            win->height = ev->xconfigure.height;
            win->resized = 1;
        }
        return;
    }
    if (ev->type == PropertyNotify) {
        if (ev->xproperty.atom == win->wm_state) {
            note_wm_state(win);
        } else if (ev->xproperty.atom == win->net_wm_state) {
            note_net_state(win);
        }
        return;
    }
    if (ev->type == UnmapNotify && !win->closing) {
        win->minimized = 1;
        suspend_grab(win);
        return;
    }
    if (ev->type == MapNotify && win->minimized) {
        win->minimized = 0;
        win->resized = 1;
        if (win->want_capture && win->focused) {
            genos_x11_set_pointer_capture(win, 1);
        }
        return;
    }
    if (ev->type == ClientMessage && (Atom)ev->xclient.data.l[0] == win->wm_delete) {
        win->closing = 1;
    }
}

static void drain(struct GenosX11 *win) {
    if (!win || !win->display) {
        return;
    }
    while (XPending(win->display)) {
        XEvent ev;
        XNextEvent(win->display, &ev);
        if (ev.type == GenericEvent && ev.xcookie.extension == win->xi_opcode) {
            if (XGetEventData(win->display, &ev.xcookie)) {
                note_xi(win, ev.xcookie.evtype, ev.xcookie.data);
                XFreeEventData(win->display, &ev.xcookie);
            }
            continue;
        }
        handle_core(win, &ev);
    }
}

static void suspend_grab(struct GenosX11 *win) {
    if (!win || !win->display) {
        return;
    }
    if (win->pointer_locked) {
        XUngrabPointer(win->display, CurrentTime);
        win->pointer_locked = 0;
    }
    if (win->cursor_hidden && win->xfixes_on) {
        XFixesShowCursor(win->display, win->window);
        win->cursor_hidden = 0;
    }
}

static void release_capture(struct GenosX11 *win) {
    win->want_capture = 0;
    suspend_grab(win);
    XFlush(win->display);
}

static int engage_capture(struct GenosX11 *win) {
    int rc;
    if (!win->display || !win->window) {
        return -1;
    }
    rc = XGrabPointer(
        win->display,
        win->window,
        True,
        ButtonPressMask | ButtonReleaseMask | PointerMotionMask,
        GrabModeAsync,
        GrabModeAsync,
        win->window,
        None,
        CurrentTime
    );
    if (rc != GrabSuccess) {
        fprintf(stderr, "genos-window: pointer capture failed %d\n", rc);
        return -1;
    }
    if (win->xfixes_on && !win->cursor_hidden) {
        XFixesHideCursor(win->display, win->window);
        win->cursor_hidden = 1;
    }
    win->pointer_locked = 1;
    XFlush(win->display);
    return 0;
}

static void select_xi(struct GenosX11 *win) {
    int event = 0;
    int error = 0;
    int major = 2;
    int minor = 2;
    unsigned char mask[XIMaskLen(XI_LASTEVENT)];
    XIEventMask events;
    if (!XQueryExtension(win->display, "XInputExtension", &win->xi_opcode, &event, &error)) {
        fprintf(stderr, "genos-window: XInput2 is missing\n");
        return;
    }
    if (XIQueryVersion(win->display, &major, &minor) != Success || major < 2) {
        fprintf(stderr, "genos-window: XInput2 is missing\n");
        win->xi_opcode = 0;
        return;
    }
    memset(mask, 0, sizeof(mask));
    XISetMask(mask, XI_RawMotion);
    events.deviceid = XIAllMasterDevices;
    events.mask_len = (int)sizeof(mask);
    events.mask = mask;
    XISelectEvents(win->display, win->root, &events, 1);
    memset(mask, 0, sizeof(mask));
    XISetMask(mask, XI_TouchBegin);
    XISetMask(mask, XI_TouchUpdate);
    XISetMask(mask, XI_TouchEnd);
    events.mask_len = (int)sizeof(mask);
    events.mask = mask;
    XISelectEvents(win->display, win->window, &events, 1);
    win->xi_on = 1;
}

static void copy_text(char *dst, size_t cap, const char *src, const char *fallback) {
    size_t i;
    if (!src || src[0] == '\0') {
        src = fallback;
    }
    for (i = 0; i + 1 < cap && src[i] != '\0'; i++) {
        dst[i] = src[i];
    }
    dst[i] = '\0';
}

struct GenosX11 *genos_x11_open(int width, int height, const char *app_id, const char *title) {
    struct GenosX11 *win;
    XSetWindowAttributes attr;
    Atom protocol;
    XClassHint *hint;
    char name_buf[128];
    char class_buf[128];
    int xfixes_event = 0;
    int xfixes_error = 0;
    Window child = 0;
    Window pointer_root = 0;
    int root_x = 0;
    int root_y = 0;
    int win_x = 0;
    int win_y = 0;
    unsigned int pointer_mask = 0;
    static int threads_ready = 0;

    if (!threads_ready) {
        XInitThreads();
        threads_ready = 1;
    }
    win = calloc(1, sizeof(*win));
    if (!win) {
        return NULL;
    }
    win->width = width > 0 ? width : 1280;
    win->height = height > 0 ? height : 720;
    win->display = XOpenDisplay(NULL);
    if (!win->display) {
        fprintf(stderr, "genos-window: XOpenDisplay failed\n");
        free(win);
        return NULL;
    }
    win->root = RootWindow(win->display, DefaultScreen(win->display));
    memset(&attr, 0, sizeof(attr));
    attr.event_mask = KeyPressMask | KeyReleaseMask | ButtonPressMask | ButtonReleaseMask |
        PointerMotionMask | FocusChangeMask | StructureNotifyMask | PropertyChangeMask;
    attr.background_pixel = BlackPixel(win->display, DefaultScreen(win->display));
    win->window = XCreateWindow(
        win->display,
        win->root,
        0,
        0,
        (unsigned)win->width,
        (unsigned)win->height,
        0,
        CopyFromParent,
        InputOutput,
        CopyFromParent,
        CWEventMask | CWBackPixel,
        &attr
    );
    if (!win->window) {
        fprintf(stderr, "genos-window: XCreateWindow failed\n");
        XCloseDisplay(win->display);
        free(win);
        return NULL;
    }
    win->wm_protocols = XInternAtom(win->display, "WM_PROTOCOLS", False);
    win->wm_delete = XInternAtom(win->display, "WM_DELETE_WINDOW", False);
    win->wm_state = XInternAtom(win->display, "WM_STATE", False);
    win->net_wm_state = XInternAtom(win->display, "_NET_WM_STATE", False);
    win->net_wm_fullscreen = XInternAtom(win->display, "_NET_WM_STATE_FULLSCREEN", False);
    protocol = win->wm_delete;
    XSetWMProtocols(win->display, win->window, &protocol, 1);
    copy_text(name_buf, sizeof(name_buf), app_id, "genos");
    copy_text(class_buf, sizeof(class_buf), app_id, "genos");
    hint = XAllocClassHint();
    if (hint) {
        hint->res_name = name_buf;
        hint->res_class = class_buf;
        XSetClassHint(win->display, win->window, hint);
        XFree(hint);
    }
    copy_text(name_buf, sizeof(name_buf), title, "Genos");
    XStoreName(win->display, win->window, name_buf);
    if (XFixesQueryExtension(win->display, &xfixes_event, &xfixes_error)) {
        win->xfixes_on = 1;
    }
    select_xi(win);
    XMapWindow(win->display, win->window);
    XFlush(win->display);
    if (XQueryPointer(
            win->display,
            win->window,
            &pointer_root,
            &child,
            &root_x,
            &root_y,
            &win_x,
            &win_y,
            &pointer_mask
        )) {
        (void)pointer_root;
        (void)child;
        (void)root_x;
        (void)root_y;
        (void)pointer_mask;
        win->last_x = win_x;
        win->last_y = win_y;
        win->have_pointer = 1;
    }
    fprintf(stderr, "genos-window: backend=x11 client=%dx%d\n", win->width, win->height);
    return win;
}

static void copy_out(struct GenosX11 *win, struct GenosPump *out) {
    memset(out, 0, sizeof(*out));
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
    out->surface = (void *)(uintptr_t)win->window;
    out->wheel_y = win->wheel_y;
    out->minimized = win->minimized;
    win->wheel_y = 0.0;
    win->mouse_dx = 0;
    win->mouse_dy = 0;
    win->capture_click = 0;
    win->clicked_while_focused = 0;
    win->resized = 0;
}

void genos_x11_pump(struct GenosX11 *win, struct GenosPump *out) {
    if (!win || !out) {
        return;
    }
    drain(win);
    copy_out(win, out);
}

void genos_x11_dispatch(struct GenosX11 *win) {
    drain(win);
}

int genos_x11_copy_extra(struct GenosX11 *win, struct GenosExtra *out) {
    int i;
    if (!out) {
        return -1;
    }
    memset(out, 0, sizeof(*out));
    if (!win) {
        return -1;
    }
    out->mouse_right = win->mouse_right;
    out->mouse_middle = win->mouse_middle;
    if (win->event_count > 0) {
        int count = win->event_count;
        if (count > GENOS_TOUCH_CAP) {
            count = GENOS_TOUCH_CAP;
        }
        out->touch_count = count;
        memcpy(out->touch, win->events, (size_t)count * sizeof(win->events[0]));
        win->event_count = 0;
        return 0;
    }
    out->touch_count = win->active_count;
    for (i = 0; i < win->active_count && i < GENOS_TOUCH_CAP; i++) {
        out->touch[i] = win->active[i];
        out->touch[i].phase = GENOS_TOUCH_MOVED;
    }
    return 0;
}

int genos_x11_set_title(struct GenosX11 *win, const char *title) {
    char name_buf[256];
    if (!win || !win->display || !title) {
        return -1;
    }
    copy_text(name_buf, sizeof(name_buf), title, "Genos");
    XStoreName(win->display, win->window, name_buf);
    XFlush(win->display);
    return 0;
}

int genos_x11_set_fullscreen(struct GenosX11 *win, int fullscreen) {
    XEvent message;
    if (!win || !win->display) {
        return -1;
    }
    memset(&message, 0, sizeof(message));
    message.xclient.type = ClientMessage;
    message.xclient.window = win->window;
    message.xclient.message_type = win->net_wm_state;
    message.xclient.format = 32;
    message.xclient.data.l[0] = fullscreen ? 1 : 0;
    message.xclient.data.l[1] = (long)win->net_wm_fullscreen;
    message.xclient.data.l[2] = 0;
    message.xclient.data.l[3] = 1;
    XSendEvent(
        win->display,
        win->root,
        False,
        SubstructureRedirectMask | SubstructureNotifyMask,
        &message
    );
    XFlush(win->display);
    return 0;
}

int genos_x11_set_pointer_capture(struct GenosX11 *win, int capture) {
    if (!win) {
        return -1;
    }
    if (!capture) {
        release_capture(win);
        return 0;
    }
    win->want_capture = 1;
    if (win->minimized) {
        return 0;
    }
    if (engage_capture(win) != 0) {
        win->want_capture = 0;
        return -1;
    }
    return 0;
}

void genos_x11_destroy(struct GenosX11 *win) {
    if (!win) {
        return;
    }
    if (win->display) {
        release_capture(win);
        if (win->window) {
            XDestroyWindow(win->display, win->window);
        }
        XCloseDisplay(win->display);
    }
    free(win);
}
