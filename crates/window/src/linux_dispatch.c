/* Linux window entry. A named Wayland display stays on Wayland.
 * Otherwise a failed Wayland connect opens X11. The Rust `Window` calls
 * stay on these names. */

#include <stdlib.h>

#include "linux_host.h"

int genos_window_backend_value = GENOS_BACKEND_WAYLAND;

struct GenosHost {
    int backend;
    void *inner;
};

int genos_window_backend(void) {
    return genos_window_backend_value;
}

static struct GenosHost *wrap(int backend, void *inner) {
    struct GenosHost *host;
    if (!inner) {
        return NULL;
    }
    host = calloc(1, sizeof(*host));
    if (!host) {
        if (backend == GENOS_BACKEND_X11) {
            genos_x11_destroy(inner);
        } else {
            genos_wayland_destroy(inner);
        }
        return NULL;
    }
    host->backend = backend;
    host->inner = inner;
    genos_window_backend_value = backend;
    return host;
}

static int named(const char *text) {
    return text && text[0] != '\0';
}

static int session_wayland(void) {
    return named(getenv("WAYLAND_DISPLAY"));
}

static int session_x11(void) {
    return named(getenv("DISPLAY"));
}

struct GenosHost *genos_window_open_on(
    int width,
    int height,
    const char *app_id,
    const char *title,
    const char *wayland_display
) {
    void *inner = NULL;
    int backend = GENOS_BACKEND_WAYLAND;
    if (named(wayland_display)) {
        inner = genos_wayland_open_on(width, height, app_id, title, wayland_display);
    } else if (session_wayland() || !session_x11()) {
        inner = genos_wayland_open(width, height, app_id, title);
        if (!inner) {
            inner = genos_x11_open(width, height, app_id, title);
            backend = GENOS_BACKEND_X11;
        }
    } else {
        inner = genos_x11_open(width, height, app_id, title);
        backend = GENOS_BACKEND_X11;
    }
    return wrap(backend, inner);
}

struct GenosHost *genos_window_open(int width, int height, const char *app_id, const char *title) {
    return genos_window_open_on(width, height, app_id, title, NULL);
}

void genos_window_pump(struct GenosHost *host, struct GenosPump *out) {
    if (!host || !out) {
        return;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        genos_x11_pump(host->inner, out);
        return;
    }
    genos_wayland_pump(host->inner, out);
}

int genos_window_pump_size(void) {
    return (int)sizeof(struct GenosPump);
}

int genos_window_extra_size(void) {
    return (int)sizeof(struct GenosExtra);
}

int genos_window_copy_extra(struct GenosHost *host, struct GenosExtra *out) {
    if (!host) {
        return -1;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        return genos_x11_copy_extra(host->inner, out);
    }
    return genos_wayland_copy_extra(host->inner, out);
}

void genos_window_dispatch(struct GenosHost *host) {
    if (!host) {
        return;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        genos_x11_dispatch(host->inner);
        return;
    }
    genos_wayland_dispatch(host->inner);
}

int genos_window_set_title(struct GenosHost *host, const char *title) {
    if (!host) {
        return -1;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        return genos_x11_set_title(host->inner, title);
    }
    return genos_wayland_set_title(host->inner, title);
}

int genos_window_set_fullscreen(struct GenosHost *host, int fullscreen) {
    if (!host) {
        return -1;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        return genos_x11_set_fullscreen(host->inner, fullscreen);
    }
    return genos_wayland_set_fullscreen(host->inner, fullscreen);
}

int genos_window_set_pointer_capture(struct GenosHost *host, int capture) {
    if (!host) {
        return -1;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        return genos_x11_set_pointer_capture(host->inner, capture);
    }
    return genos_wayland_set_pointer_capture(host->inner, capture);
}

void genos_window_destroy(struct GenosHost *host) {
    if (!host) {
        return;
    }
    if (host->backend == GENOS_BACKEND_X11) {
        genos_x11_destroy(host->inner);
    } else {
        genos_wayland_destroy(host->inner);
    }
    free(host);
}
