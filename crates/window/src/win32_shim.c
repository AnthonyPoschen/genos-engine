/* Win32 window. The Rust `Window` API is unchanged.
 * `display` is the HINSTANCE. `surface` is the HWND.
 * The renderer creates the VkSurface with VK_KHR_win32_surface.
 * Keys in `keys_down` are Linux evdev codes from the set-1 scan code
 * (`genos_evdev_from_set1`), so left and right modifiers stay distinct. */

#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0A00
#endif
#ifndef WINVER
#define WINVER 0x0A00
#endif
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>

#include "genos_pump.h"

struct GenosWin32 {
    HINSTANCE instance;
    HWND hwnd;
    int width;
    int height;
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
    int mouse_right;
    int mouse_middle;
    struct GenosTouch events[GENOS_TOUCH_CAP];
    int event_count;
    struct GenosTouch active[GENOS_TOUCH_CAP];
    int active_count;
    uint8_t keys_down[256];
    int pointer_locked;
    int clicked_while_focused;
    int fullscreen;
    double wheel_y;
    int cursor_hidden;
    int raw_on;
    int want_capture;
    int minimized;
    int buttons_held;
    int abs_ready;
    int abs_x;
    int abs_y;
    unsigned char *raw_buf;
    UINT raw_cap;
    DWORD thread;
    LONG_PTR saved_style;
    RECT saved_rect;
    wchar_t class_name[64];
};

static void release_capture(struct GenosWin32 *win);
static void suspend_grab(struct GenosWin32 *win);
static int engage_capture(struct GenosWin32 *win);
static void clip_client(struct GenosWin32 *win);

extern int genos_evdev_from_set1(unsigned scan, int extended);
extern int genos_absolute_pixel(int norm, int origin, int span);
extern int genos_absolute_delta(int ready, int prev, int next);
static LRESULT CALLBACK window_proc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp);

/* Copy a `GetProcAddress` result without a function-pointer cast. */
static void load_user(void *dst, size_t bytes, const char *name) {
    HMODULE user = GetModuleHandleW(L"user32.dll");
    FARPROC proc = NULL;
    if (!dst || bytes == 0) {
        return;
    }
    memset(dst, 0, bytes);
    if (!user || bytes != sizeof(proc)) {
        return;
    }
    proc = GetProcAddress(user, name);
    if (proc) {
        memcpy(dst, &proc, sizeof(proc));
    }
}

static void enable_dpi(void) {
    typedef BOOL(WINAPI *SetFn)(HANDLE);
    SetFn set_ctx = NULL;
    load_user(&set_ctx, sizeof(set_ctx), "SetProcessDpiAwarenessContext");
    /* DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 is (HANDLE)-4. */
    if (set_ctx && set_ctx((HANDLE)(intptr_t)-4)) {
        return;
    }
    SetProcessDPIAware();
    fprintf(stderr, "genos-window: dpi=legacy\n");
}

static UINT window_dpi(HWND hwnd) {
    if (hwnd) {
        typedef UINT(WINAPI *ForWindow)(HWND);
        ForWindow get_dpi = NULL;
        load_user(&get_dpi, sizeof(get_dpi), "GetDpiForWindow");
        if (get_dpi) {
            UINT dots = get_dpi(hwnd);
            if (dots > 0) {
                return dots;
            }
        }
    }
    {
        typedef UINT(WINAPI *ForSystem)(void);
        ForSystem get_sys = NULL;
        load_user(&get_sys, sizeof(get_sys), "GetDpiForSystem");
        if (get_sys) {
            UINT dots = get_sys();
            if (dots > 0) {
                return dots;
            }
        }
    }
    return 96;
}

static int to_wide(const char *utf8, wchar_t *out, int cap) {
    int wrote;
    if (!utf8 || !out || cap < 2) {
        return 0;
    }
    wrote = MultiByteToWideChar(CP_UTF8, 0, utf8, -1, out, cap);
    return wrote > 0;
}

static int evdev_from_lparam(LPARAM lp) {
    unsigned scan = (unsigned)((lp >> 16) & 0xFF);
    int extended = (lp & 0x01000000) != 0;
    return genos_evdev_from_set1(scan, extended);
}

static void set_evdev(struct GenosWin32 *win, int evdev, int down) {
    if (!win || evdev < 0 || evdev > 255) {
        return;
    }
    win->keys_down[evdev] = (uint8_t)(down ? 1 : 0);
    if (evdev == 17) win->key_w = down;
    if (evdev == 30) win->key_a = down;
    if (evdev == 31) win->key_s = down;
    if (evdev == 32) win->key_d = down;
    if (evdev == 1) {
        win->key_escape = down;
        /* The Wayland seat releases the pointer when Escape goes down. */
        if (down) {
            release_capture(win);
        }
    }
}

static void clear_keys(struct GenosWin32 *win) {
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

static void push_touch(struct GenosWin32 *win, uint64_t id, float x, float y, int phase) {
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

static void cancel_touch(struct GenosWin32 *win) {
    while (win->active_count > 0) {
        struct GenosTouch sample = win->active[0];
        push_touch(win, sample.id, sample.x, sample.y, GENOS_TOUCH_CANCELLED);
    }
}

#ifdef WM_POINTERDOWN
static void note_pointer(struct GenosWin32 *win, UINT msg, WPARAM wp) {
    POINTER_INFO info;
    POINT point;
    UINT32 id;
    int phase;
    memset(&info, 0, sizeof(info));
    id = GET_POINTERID_WPARAM(wp);
    if (!GetPointerInfo(id, &info)) {
        return;
    }
    if (info.pointerType != PT_TOUCH && info.pointerType != PT_PEN) {
        return;
    }
    point = info.ptPixelLocation;
    ScreenToClient(win->hwnd, &point);
    if (msg == WM_POINTERDOWN) {
        phase = GENOS_TOUCH_BEGAN;
    } else if (msg == WM_POINTERUP) {
        phase = GENOS_TOUCH_ENDED;
    } else if (msg == WM_POINTERCAPTURECHANGED) {
        phase = GENOS_TOUCH_CANCELLED;
    } else {
        phase = GENOS_TOUCH_MOVED;
    }
    push_touch(win, (uint64_t)id, (float)point.x, (float)point.y, phase);
}
#endif

static void clip_client(struct GenosWin32 *win) {
    RECT rect;
    POINT origin;
    if (!win || !win->hwnd) {
        return;
    }
    if (!GetClientRect(win->hwnd, &rect)) {
        return;
    }
    origin.x = rect.left;
    origin.y = rect.top;
    ClientToScreen(win->hwnd, &origin);
    rect.right += origin.x - rect.left;
    rect.bottom += origin.y - rect.top;
    rect.left = origin.x;
    rect.top = origin.y;
    ClipCursor(&rect);
}

static void suspend_grab(struct GenosWin32 *win) {
    RAWINPUTDEVICE raw;
    if (!win) {
        return;
    }
    ClipCursor(NULL);
    if (win->cursor_hidden) {
        ShowCursor(TRUE);
        win->cursor_hidden = 0;
    }
    if (win->raw_on) {
        memset(&raw, 0, sizeof(raw));
        raw.usUsagePage = 0x01;
        raw.usUsage = 0x02;
        raw.dwFlags = RIDEV_REMOVE;
        raw.hwndTarget = NULL;
        RegisterRawInputDevices(&raw, 1, sizeof(raw));
        win->raw_on = 0;
    }
    win->pointer_locked = 0;
    win->abs_ready = 0;
}

static void release_capture(struct GenosWin32 *win) {
    if (!win) {
        return;
    }
    win->want_capture = 0;
    suspend_grab(win);
}

static int engage_capture(struct GenosWin32 *win) {
    RAWINPUTDEVICE raw;
    if (!win || !win->hwnd) {
        return -1;
    }
    memset(&raw, 0, sizeof(raw));
    raw.usUsagePage = 0x01;
    raw.usUsage = 0x02;
    raw.dwFlags = 0;
    raw.hwndTarget = win->hwnd;
    if (!RegisterRawInputDevices(&raw, 1, sizeof(raw))) {
        fprintf(stderr, "genos-window: raw mouse failed %lu\n", GetLastError());
        return -1;
    }
    win->raw_on = 1;
    if (!win->cursor_hidden) {
        ShowCursor(FALSE);
        win->cursor_hidden = 1;
    }
    clip_client(win);
    win->pointer_locked = 1;
    return 0;
}

static void note_mouse_button(struct GenosWin32 *win, int which, int down) {
    int *flag;
    if (!win) {
        return;
    }
    if (which == 0) {
        flag = &win->mouse_left;
    } else if (which == 1) {
        flag = &win->mouse_right;
    } else {
        flag = &win->mouse_middle;
    }
    if (down) {
        if (!*flag) {
            if (win->buttons_held == 0 && win->hwnd) {
                SetCapture(win->hwnd);
            }
            win->buttons_held += 1;
        }
        *flag = 1;
        if (which == 0) {
            win->capture_click = 1;
            /* The click hit this window. Capture even if keyboard focus is late. */
            win->clicked_while_focused = 1;
        }
        return;
    }
    if (*flag && win->buttons_held > 0) {
        win->buttons_held -= 1;
        if (win->buttons_held == 0) {
            ReleaseCapture();
        }
    }
    *flag = 0;
}

static void read_raw_mouse(struct GenosWin32 *win, LPARAM lp) {
    UINT size = 0;
    RAWINPUT *raw;
    USHORT flags;
    if (GetRawInputData((HRAWINPUT)lp, RID_INPUT, NULL, &size, sizeof(RAWINPUTHEADER)) != 0
        || size < sizeof(RAWINPUTHEADER)) {
        return;
    }
    if (size > win->raw_cap) {
        unsigned char *grown = (unsigned char *)realloc(win->raw_buf, size);
        if (!grown) {
            return;
        }
        win->raw_buf = grown;
        win->raw_cap = size;
    }
    if (GetRawInputData((HRAWINPUT)lp, RID_INPUT, win->raw_buf, &size, sizeof(RAWINPUTHEADER))
        == (UINT)-1) {
        return;
    }
    raw = (RAWINPUT *)win->raw_buf;
    if (raw->header.dwType != RIM_TYPEMOUSE) {
        return;
    }
    flags = raw->data.mouse.usFlags;
    if (flags & MOUSE_MOVE_ABSOLUTE) {
        int origin_x = 0;
        int origin_y = 0;
        int span_x = GetSystemMetrics(SM_CXSCREEN);
        int span_y = GetSystemMetrics(SM_CYSCREEN);
        int x;
        int y;
        if (flags & MOUSE_VIRTUAL_DESKTOP) {
            origin_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            origin_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            span_x = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            span_y = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        }
        x = genos_absolute_pixel((int)raw->data.mouse.lLastX, origin_x, span_x);
        y = genos_absolute_pixel((int)raw->data.mouse.lLastY, origin_y, span_y);
        win->mouse_dx += genos_absolute_delta(win->abs_ready, win->abs_x, x);
        win->mouse_dy += genos_absolute_delta(win->abs_ready, win->abs_y, y);
        win->abs_x = x;
        win->abs_y = y;
        win->abs_ready = 1;
        return;
    }
    win->mouse_dx += (int)raw->data.mouse.lLastX;
    win->mouse_dy += (int)raw->data.mouse.lLastY;
}

static void note_move(struct GenosWin32 *win, int x, int y) {
    if (!win->pointer_locked && win->have_pointer) {
        win->mouse_dx += x - win->last_x;
        win->mouse_dy += y - win->last_y;
    }
    win->last_x = x;
    win->last_y = y;
    win->have_pointer = 1;
}

static LRESULT CALLBACK window_proc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    struct GenosWin32 *win;
    if (msg == WM_NCCREATE) {
        CREATESTRUCTW *created = (CREATESTRUCTW *)lp;
        win = (struct GenosWin32 *)created->lpCreateParams;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (LONG_PTR)win);
        if (win) {
            win->hwnd = hwnd;
        }
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    win = (struct GenosWin32 *)GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if (!win) {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    switch (msg) {
    case WM_CLOSE:
        win->closing = 1;
        return 0;
    case WM_SETFOCUS:
        win->focused = 1;
        if (win->want_capture && !win->pointer_locked && !win->minimized) {
            engage_capture(win);
        }
        return 0;
    case WM_KILLFOCUS:
        win->focused = 0;
        clear_keys(win);
        cancel_touch(win);
        win->buttons_held = 0;
        ReleaseCapture();
        /* Drop the clip and show the cursor. `want_capture` stays so focus can re-grab. */
        suspend_grab(win);
        return 0;
    case WM_SIZE:
        if (wp == SIZE_MINIMIZED) {
            win->minimized = 1;
            suspend_grab(win);
            return 0;
        }
        if (win->minimized) {
            win->minimized = 0;
            win->resized = 1;
            if (win->want_capture && win->focused) {
                engage_capture(win);
            }
        }
        {
            int width = LOWORD(lp);
            int height = HIWORD(lp);
            if (width > 0 && height > 0 && (width != win->width || height != win->height)) {
                win->width = width;
                win->height = height;
                win->resized = 1;
            }
            if (win->pointer_locked) {
                clip_client(win);
            }
        }
        return 0;
    case WM_DPICHANGED: {
        RECT *hint = (RECT *)lp;
        if (hint) {
            SetWindowPos(
                hwnd,
                NULL,
                hint->left,
                hint->top,
                hint->right - hint->left,
                hint->bottom - hint->top,
                SWP_NOZORDER | SWP_NOACTIVATE
            );
        }
        return 0;
    }
    case WM_MOUSEMOVE:
        note_move(win, (int)(short)LOWORD(lp), (int)(short)HIWORD(lp));
        return 0;
    case WM_LBUTTONDOWN:
        note_mouse_button(win, 0, 1);
        return 0;
    case WM_LBUTTONUP:
        note_mouse_button(win, 0, 0);
        return 0;
    case WM_RBUTTONDOWN:
        note_mouse_button(win, 1, 1);
        return 0;
    case WM_RBUTTONUP:
        note_mouse_button(win, 1, 0);
        return 0;
    case WM_MBUTTONDOWN:
        note_mouse_button(win, 2, 1);
        return 0;
    case WM_MBUTTONUP:
        note_mouse_button(win, 2, 0);
        return 0;
    case WM_CAPTURECHANGED:
        if ((HWND)lp != hwnd) {
            win->buttons_held = 0;
        }
        return 0;
#ifdef WM_POINTERDOWN
    case WM_POINTERDOWN:
    case WM_POINTERUPDATE:
    case WM_POINTERUP:
    case WM_POINTERCAPTURECHANGED:
        note_pointer(win, msg, wp);
        return 0;
#endif
    case WM_MOUSEWHEEL:
        /* Positive HIWORD is wheel up, which is the frame's sign. */
        win->wheel_y += (double)(short)HIWORD(wp) / 120.0;
        return 0;
    case WM_KEYDOWN:
    case WM_SYSKEYDOWN:
        set_evdev(win, evdev_from_lparam(lp), 1);
        return 0;
    case WM_KEYUP:
    case WM_SYSKEYUP:
        set_evdev(win, evdev_from_lparam(lp), 0);
        return 0;
    case WM_INPUT:
        if (win->pointer_locked) {
            read_raw_mouse(win, lp);
        }
        return 0;
    default:
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
}

static void drain(struct GenosWin32 *win) {
    MSG msg;
    if (!win || GetCurrentThreadId() != win->thread) {
        return;
    }
    while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

static void copy_out(struct GenosWin32 *win, struct GenosPump *out) {
    memset(out, 0, sizeof(*out));
    out->width = win->width > 0 ? win->width : 1;
    out->height = win->height > 0 ? win->height : 1;
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
    out->display = win->instance;
    out->surface = win->hwnd;
    out->wheel_y = win->wheel_y;
    out->minimized = win->minimized;
    win->wheel_y = 0;
    win->mouse_dx = 0;
    win->mouse_dy = 0;
    win->capture_click = 0;
    win->clicked_while_focused = 0;
    win->resized = 0;
}

static int register_class(struct GenosWin32 *win, const wchar_t *name) {
    WNDCLASSEXW wc;
    memset(&wc, 0, sizeof(wc));
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = window_proc;
    wc.hInstance = win->instance;
    /* 32512 is IDC_ARROW. The W macro stays a wide pointer when UNICODE is off. */
    wc.hCursor = LoadCursorW(NULL, MAKEINTRESOURCEW(32512));
    wc.lpszClassName = name;
    wc.style = CS_HREDRAW | CS_VREDRAW;
    if (RegisterClassExW(&wc) != 0) {
        return 1;
    }
    return GetLastError() == ERROR_CLASS_ALREADY_EXISTS;
}

struct GenosWin32 *genos_window_open_on(
    int width,
    int height,
    const char *app_id,
    const char *title,
    const char *wayland_display
) {
    struct GenosWin32 *win;
    wchar_t title_w[256];
    RECT rect;
    DWORD style = WS_OVERLAPPEDWINDOW;
    UINT dots;
    (void)wayland_display;
    enable_dpi();
    win = (struct GenosWin32 *)calloc(1, sizeof(*win));
    if (!win) {
        return NULL;
    }
    win->instance = GetModuleHandleW(NULL);
    win->thread = GetCurrentThreadId();
    win->width = width > 0 ? width : 1280;
    win->height = height > 0 ? height : 720;
    if (!app_id || app_id[0] == '\0') {
        app_id = "genos";
    }
    if (!title || title[0] == '\0') {
        title = "Genos";
    }
    if (!to_wide(app_id, win->class_name, 64) || !to_wide(title, title_w, 256)) {
        fprintf(stderr, "genos-window: title is not utf-8\n");
        free(win);
        return NULL;
    }
    if (!register_class(win, win->class_name)) {
        fprintf(stderr, "genos-window: RegisterClass failed %lu\n", GetLastError());
        free(win);
        return NULL;
    }
    rect.left = 0;
    rect.top = 0;
    rect.right = win->width;
    rect.bottom = win->height;
    dots = window_dpi(NULL);
    {
        typedef BOOL(WINAPI *AdjustFn)(LPRECT, DWORD, BOOL, DWORD, UINT);
        AdjustFn adjust = NULL;
        load_user(&adjust, sizeof(adjust), "AdjustWindowRectExForDpi");
        if (!adjust || !adjust(&rect, style, FALSE, 0, dots)) {
            AdjustWindowRectEx(&rect, style, FALSE, 0);
        }
    }
    win->hwnd = CreateWindowExW(
        0,
        win->class_name,
        title_w,
        style | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        rect.right - rect.left,
        rect.bottom - rect.top,
        NULL,
        NULL,
        win->instance,
        win
    );
    if (!win->hwnd) {
        fprintf(stderr, "genos-window: CreateWindow failed %lu\n", GetLastError());
        free(win);
        return NULL;
    }
    {
        RECT client;
        if (GetClientRect(win->hwnd, &client) && client.right > 0 && client.bottom > 0) {
            win->width = client.right;
            win->height = client.bottom;
        }
    }
    dots = window_dpi(win->hwnd);
    fprintf(stderr, "genos-window: backend=win32 dpi=%u client=%dx%d\n", dots, win->width, win->height);
    return win;
}

struct GenosWin32 *genos_window_open(int width, int height, const char *app_id, const char *title) {
    return genos_window_open_on(width, height, app_id, title, NULL);
}

void genos_window_pump(struct GenosWin32 *win, struct GenosPump *out) {
    if (!win || !out) {
        return;
    }
    drain(win);
    copy_out(win, out);
}

int genos_window_pump_size(void) {
    return (int)sizeof(struct GenosPump);
}

int genos_window_extra_size(void) {
    return (int)sizeof(struct GenosExtra);
}

int genos_window_copy_extra(struct GenosWin32 *win, struct GenosExtra *out) {
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

void genos_window_dispatch(struct GenosWin32 *win) {
    drain(win);
}

int genos_window_set_title(struct GenosWin32 *win, const char *title) {
    wchar_t wide[256];
    if (!win || !win->hwnd || !title) {
        return -1;
    }
    if (!to_wide(title, wide, 256)) {
        return -1;
    }
    return SetWindowTextW(win->hwnd, wide) ? 0 : -1;
}

int genos_window_set_fullscreen(struct GenosWin32 *win, int fullscreen) {
    MONITORINFO info;
    HMONITOR monitor;
    if (!win || !win->hwnd) {
        return -1;
    }
    if (fullscreen && !win->fullscreen) {
        if (!GetWindowRect(win->hwnd, &win->saved_rect)) {
            fprintf(stderr, "genos-window: window rect failed\n");
            return -1;
        }
        win->saved_style = GetWindowLongPtrW(win->hwnd, GWL_STYLE);
        monitor = MonitorFromWindow(win->hwnd, MONITOR_DEFAULTTONEAREST);
        memset(&info, 0, sizeof(info));
        info.cbSize = sizeof(info);
        if (!GetMonitorInfoW(monitor, &info)) {
            fprintf(stderr, "genos-window: monitor info failed\n");
            return -1;
        }
        SetWindowLongPtrW(win->hwnd, GWL_STYLE, (win->saved_style & ~WS_OVERLAPPEDWINDOW) | WS_POPUP);
        SetWindowPos(
            win->hwnd,
            HWND_TOP,
            info.rcMonitor.left,
            info.rcMonitor.top,
            info.rcMonitor.right - info.rcMonitor.left,
            info.rcMonitor.bottom - info.rcMonitor.top,
            SWP_FRAMECHANGED | SWP_SHOWWINDOW
        );
        win->fullscreen = 1;
        return 0;
    }
    if (!fullscreen && win->fullscreen) {
        SetWindowLongPtrW(win->hwnd, GWL_STYLE, win->saved_style);
        SetWindowPos(
            win->hwnd,
            HWND_NOTOPMOST,
            win->saved_rect.left,
            win->saved_rect.top,
            win->saved_rect.right - win->saved_rect.left,
            win->saved_rect.bottom - win->saved_rect.top,
            SWP_FRAMECHANGED | SWP_SHOWWINDOW
        );
        win->fullscreen = 0;
    }
    return 0;
}

int genos_window_set_pointer_capture(struct GenosWin32 *win, int capture) {
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
    return engage_capture(win);
}

void genos_window_destroy(struct GenosWin32 *win) {
    if (!win) {
        return;
    }
    release_capture(win);
    if (win->hwnd) {
        DestroyWindow(win->hwnd);
        win->hwnd = NULL;
    }
    free(win->raw_buf);
    free(win);
}

int genos_window_backend(void) {
    return GENOS_BACKEND_WIN32;
}
