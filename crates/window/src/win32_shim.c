/* Win32 window. The Rust `Window` API is unchanged.
 * `display` is the HINSTANCE. `surface` is the HWND.
 * The renderer creates the VkSurface with VK_KHR_win32_surface.
 * Keys in `keys_down` are Linux evdev codes, the same index the Wayland seat writes. */

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
    DWORD thread;
    LONG_PTR saved_style;
    RECT saved_rect;
    wchar_t class_name[64];
};

static void release_capture(struct GenosWin32 *win);
static int engage_capture(struct GenosWin32 *win);
static void clip_client(struct GenosWin32 *win);
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

static int evdev_from_vk(WPARAM vk) {
    if (vk >= 'A' && vk <= 'Z') {
        static const int letter[26] = {
            30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50,
            49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44
        };
        return letter[vk - 'A'];
    }
    if (vk >= '1' && vk <= '9') {
        return 2 + (int)(vk - '1');
    }
    switch (vk) {
    case '0': return 11;
    case VK_ESCAPE: return 1;
    case VK_OEM_4: return 26;
    case VK_OEM_6: return 27;
    case VK_OEM_3: return 41;
    case VK_BACK: return 14;
    case VK_TAB: return 15;
    case VK_RETURN: return 28;
    case VK_SPACE: return 57;
    case VK_SHIFT:
    case VK_LSHIFT: return 42;
    case VK_RSHIFT: return 54;
    case VK_CONTROL:
    case VK_LCONTROL: return 29;
    case VK_RCONTROL: return 97;
    case VK_MENU:
    case VK_LMENU: return 56;
    case VK_RMENU: return 100;
    case VK_UP: return 103;
    case VK_DOWN: return 108;
    case VK_LEFT: return 105;
    case VK_RIGHT: return 106;
    case VK_F1: return 59;
    case VK_F2: return 60;
    case VK_F3: return 61;
    case VK_F4: return 62;
    case VK_F5: return 63;
    case VK_F6: return 64;
    case VK_F7: return 65;
    case VK_F8: return 66;
    case VK_F9: return 67;
    case VK_F10: return 68;
    case VK_F11: return 87;
    case VK_F12: return 88;
    default: return -1;
    }
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

static void release_capture(struct GenosWin32 *win) {
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

static void note_button(struct GenosWin32 *win, int down) {
    win->mouse_left = down;
    if (down) {
        win->capture_click = 1;
        /* The click hit this window. Capture even if keyboard focus is late. */
        win->clicked_while_focused = 1;
    }
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
        return 0;
    case WM_KILLFOCUS:
        win->focused = 0;
        clear_keys(win);
        cancel_touch(win);
        return 0;
    case WM_SIZE:
        if (wp != SIZE_MINIMIZED) {
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
        note_button(win, 1);
        return 0;
    case WM_LBUTTONUP:
        note_button(win, 0);
        return 0;
    case WM_RBUTTONDOWN:
        win->mouse_right = 1;
        return 0;
    case WM_RBUTTONUP:
        win->mouse_right = 0;
        return 0;
    case WM_MBUTTONDOWN:
        win->mouse_middle = 1;
        return 0;
    case WM_MBUTTONUP:
        win->mouse_middle = 0;
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
        set_evdev(win, evdev_from_vk(wp), 1);
        return 0;
    case WM_KEYUP:
    case WM_SYSKEYUP:
        set_evdev(win, evdev_from_vk(wp), 0);
        return 0;
    case WM_INPUT:
        if (win->pointer_locked) {
            UINT size = sizeof(RAWINPUT);
            RAWINPUT raw;
            memset(&raw, 0, sizeof(raw));
            if (GetRawInputData((HRAWINPUT)lp, RID_INPUT, &raw, &size, sizeof(RAWINPUTHEADER)) != (UINT)-1
                && raw.header.dwType == RIM_TYPEMOUSE) {
                win->mouse_dx += raw.data.mouse.lLastX;
                win->mouse_dy += raw.data.mouse.lLastY;
            }
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
    free(win);
}

int genos_window_backend(void) {
    return GENOS_BACKEND_WIN32;
}
