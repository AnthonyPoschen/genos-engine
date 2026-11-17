#ifndef GENOS_LINUX_HOST_H
#define GENOS_LINUX_HOST_H

#include "genos_pump.h"

/* Wayland and X11 keep their own windows. `linux_dispatch.c` picks one. */

struct GenosWindow;
struct GenosX11;

struct GenosWindow *genos_wayland_open_on(
    int width,
    int height,
    const char *app_id,
    const char *title,
    const char *wayland_display
);
struct GenosWindow *genos_wayland_open(
    int width,
    int height,
    const char *app_id,
    const char *title
);
void genos_wayland_pump(struct GenosWindow *win, struct GenosPump *out);
void genos_wayland_dispatch(struct GenosWindow *win);
int genos_wayland_set_title(struct GenosWindow *win, const char *title);
int genos_wayland_set_fullscreen(struct GenosWindow *win, int fullscreen);
int genos_wayland_set_pointer_capture(struct GenosWindow *win, int capture);
void genos_wayland_destroy(struct GenosWindow *win);
int genos_wayland_copy_extra(void *window, struct GenosExtra *out);

struct GenosX11 *genos_x11_open(int width, int height, const char *app_id, const char *title);
void genos_x11_pump(struct GenosX11 *win, struct GenosPump *out);
void genos_x11_dispatch(struct GenosX11 *win);
int genos_x11_set_title(struct GenosX11 *win, const char *title);
int genos_x11_set_fullscreen(struct GenosX11 *win, int fullscreen);
int genos_x11_set_pointer_capture(struct GenosX11 *win, int capture);
void genos_x11_destroy(struct GenosX11 *win);
int genos_x11_copy_extra(struct GenosX11 *win, struct GenosExtra *out);

#endif
